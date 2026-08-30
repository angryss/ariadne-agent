import { describe, expect, it, vi } from 'vitest';

import { HttpAgentClient } from './httpAgentClient';

describe('HttpAgentClient', () => {
  it('posts a response request to the Rynna API', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          message: { role: 'assistant', content: 'From the server.' },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);
    const request = {
      prompt: 'Hello',
      history: [{ role: 'user' as const, content: 'Earlier' }],
    };

    const response = await client.respond(request);

    expect(fetcher).toHaveBeenCalledWith('/v1/respond', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
    });
    expect(response.message.content).toBe('From the server.');
  });

  it('streams typed thinking and content events from the Rynna API', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        [
          'data: {"kind":"thinking","content":"Inspect"}\n\n',
          'data: {"kind":"content","content":"Answer"}\n\n',
          'data: {"kind":"done","message":{"role":"assistant","content":"Answer"}}\n\n',
        ].join(''),
        { status: 200, headers: { 'content-type': 'text/event-stream' } },
      ),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);
    const deltas: unknown[] = [];
    const request = { prompt: 'Hello', history: [] };

    const response = await client.respond(request, (delta) => deltas.push(delta));

    expect(fetcher).toHaveBeenCalledWith('/v1/respond/stream', {
      method: 'POST',
      headers: {
        accept: 'text/event-stream',
        'content-type': 'application/json',
      },
      body: JSON.stringify(request),
    });
    expect(deltas).toEqual([
      { kind: 'thinking', content: 'Inspect' },
      { kind: 'content', content: 'Answer' },
    ]);
    expect(response.message.content).toBe('Answer');
  });

  it('reports an HTTP status when an error response is not JSON', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response('Bad Gateway', {
        status: 502,
        headers: { 'content-type': 'text/plain' },
      }),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);

    await expect(client.respond({ prompt: 'Hello', history: [] })).rejects.toThrow(
      'Rynna API returned 502',
    );
  });

  it('loads profile metadata from the profiles endpoint', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          default_profile: 'local',
          profiles: [
            {
              name: 'local',
              provider: 'ollama',
              model: 'qwen3:8b',
              active_skills: ['rust'],
              mcp_servers: [],
              capabilities: ['workspace'],
            },
          ],
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);

    const profiles = await client.listProfiles();

    expect(fetcher).toHaveBeenCalledWith('/v1/profiles', {
      method: 'GET',
      headers: { accept: 'application/json' },
    });
    expect(profiles.default_profile).toBe('local');
    expect(profiles.profiles[0]!.active_skills).toEqual(['rust']);
  });

  it('lists and mutates provider settings through the providers API', async () => {
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse([]))
      .mockResolvedValueOnce(jsonResponse({ kind: 'ollama', api_base: 'http://localhost:11434/v1' }))
      .mockResolvedValueOnce(jsonResponse({ kind: 'ollama', api_base: 'http://localhost:22434/v1' }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new HttpAgentClient('/v1/respond', fetcher);

    await expect(client.listProviders()).resolves.toEqual([]);
    await client.createProvider({ kind: 'ollama', api_base: 'http://localhost:11434/v1' });
    await client.updateProvider({ kind: 'ollama', api_base: 'http://localhost:22434/v1' });
    await client.deleteProvider('ollama');

    expect(fetcher).toHaveBeenNthCalledWith(1, '/v1/providers', expect.objectContaining({ method: 'GET' }));
    expect(fetcher).toHaveBeenNthCalledWith(2, '/v1/providers', expect.objectContaining({ method: 'POST' }));
    expect(fetcher).toHaveBeenNthCalledWith(3, '/v1/providers/ollama', expect.objectContaining({ method: 'PUT' }));
    expect(fetcher).toHaveBeenNthCalledWith(4, '/v1/providers/ollama', expect.objectContaining({ method: 'DELETE' }));
  });

  it('discovers an existing ChatGPT subscription through the providers API', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      jsonResponse({ connected: true, method: 'chatgpt' }),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);

    await expect(client.getExistingOpenAiAccount()).resolves.toEqual({
      connected: true,
      method: 'chatgpt',
    });
    expect(fetcher).toHaveBeenCalledWith('/v1/providers/openai/existing-account', {
      method: 'GET',
      headers: { accept: 'application/json' },
    });
  });
});

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}
