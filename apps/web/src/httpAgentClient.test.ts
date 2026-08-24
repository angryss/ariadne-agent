import { describe, expect, it, vi } from 'vitest';

import { HttpAgentClient } from './httpAgentClient';

describe('HttpAgentClient', () => {
  it('posts a response request to the Ariadne API', async () => {
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

  it('reports an HTTP status when an error response is not JSON', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response('Bad Gateway', {
        status: 502,
        headers: { 'content-type': 'text/plain' },
      }),
    );
    const client = new HttpAgentClient('/v1/respond', fetcher);

    await expect(client.respond({ prompt: 'Hello', history: [] })).rejects.toThrow(
      'Ariadne API returned 502',
    );
  });
});
