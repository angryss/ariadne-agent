import { describe, expect, it, vi } from 'vitest';

import { TauriAgentClient } from './tauriAgentClient';

describe('TauriAgentClient', () => {
  it('invokes the narrow desktop response command', async () => {
    const invoke = vi.fn().mockResolvedValue({
      message: { role: 'assistant', content: 'From Tauri.' },
    });
    const client = new TauriAgentClient(invoke);
    const request = { prompt: 'Hello', history: [] };

    const response = await client.respond(request);

    expect(invoke).toHaveBeenCalledWith('respond', { request });
    expect(response.message.content).toBe('From Tauri.');
  });

  it('streams typed thinking and content through a narrow Tauri channel', async () => {
    const channel = {
      onmessage: null as ((message: { kind: 'thinking' | 'content'; content: string }) => void) | null,
    };
    const invoke = vi.fn().mockImplementation(async (_command, args) => {
      channel.onmessage?.({ kind: 'thinking', content: 'Inspect' });
      channel.onmessage?.({ kind: 'content', content: 'Answer' });
      expect(args.onEvent).toBe(channel);
      return { message: { role: 'assistant', content: 'Answer' } };
    });
    const client = new TauriAgentClient(invoke, () => channel);
    const deltas: unknown[] = [];
    const request = { prompt: 'Hello', history: [] };

    const response = await client.respond(request, (delta) => deltas.push(delta));

    expect(invoke).toHaveBeenCalledWith('respond_stream', { request, onEvent: channel });
    expect(deltas).toEqual([
      { kind: 'thinking', content: 'Inspect' },
      { kind: 'content', content: 'Answer' },
    ]);
    expect(response.message.content).toBe('Answer');
  });

  it('loads profiles through the narrow desktop profiles command', async () => {
    const invoke = vi.fn().mockResolvedValue({
      default_profile: 'local',
      provider_ids: ['ollama', 'unused-custom'],
      profiles: [
        {
          name: 'local',
          providers: [{ provider: 'ollama', model: 'qwen3:8b' }],
          active_skills: [],
          mcp_servers: ['filesystem'],
          capabilities: ['workspace'],
        },
      ],
    });
    const client = new TauriAgentClient(invoke);

    const profiles = await client.listProfiles();

    expect(invoke).toHaveBeenCalledWith('profiles', {});
    expect(profiles.default_profile).toBe('local');
    expect(profiles.provider_ids).toEqual(['ollama', 'unused-custom']);
    expect(profiles.profiles[0]!.mcp_servers).toEqual(['filesystem']);
  });

  it('rejects profile metadata that omits catalog provider identifiers', async () => {
    const invoke = vi.fn().mockResolvedValue({
      default_profile: 'local',
      profiles: [
        {
          name: 'local',
          providers: [{ provider: 'ollama', model: 'qwen3:8b' }],
          active_skills: [],
          mcp_servers: [],
          capabilities: [],
        },
      ],
    });

    await expect(new TauriAgentClient(invoke).listProfiles()).rejects.toThrow(
      'invalid profile data',
    );
  });

  it('uses narrow commands for profile CRUD', async () => {
    const profile = {
      name: 'work',
      providers: [{ provider: 'openai', model: 'gpt-5' }],
      active_skills: [],
      mcp_servers: [],
      capabilities: [],
    };
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(profile)
      .mockResolvedValueOnce({
        ...profile,
        providers: [{ provider: 'openai', model: 'gpt-5.2' }],
      })
      .mockResolvedValueOnce(undefined);
    const client = new TauriAgentClient(invoke);

    await client.createProfile(profile);
    await client.updateProfile('work', {
      ...profile,
      providers: [{ provider: 'openai', model: 'gpt-5.2' }],
    });
    await client.deleteProfile('work');

    expect(invoke).toHaveBeenNthCalledWith(1, 'create_profile', { profile });
    expect(invoke).toHaveBeenNthCalledWith(2, 'update_profile', {
      name: 'work',
      profile: {
        ...profile,
        providers: [{ provider: 'openai', model: 'gpt-5.2' }],
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, 'delete_profile', { name: 'work' });
  });

  it('uses narrow commands for OpenAI account status and login', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({ connected: false, method: null })
      .mockResolvedValueOnce({ connected: true, method: 'chatgpt', plan: 'plus' })
      .mockResolvedValueOnce({ connected: true, method: 'api_key' });
    const client = new TauriAgentClient(invoke);

    await expect(client.getOpenAiAccount()).resolves.toEqual({ connected: false, method: null });
    await expect(client.getExistingOpenAiAccount()).resolves.toEqual({
      connected: true,
      method: 'chatgpt',
      plan: 'plus',
    });
    await expect(
      client.connectOpenAi({ method: 'api_key', api_key: 'sk-secret' }),
    ).resolves.toEqual({ connected: true, method: 'api_key' });

    expect(invoke).toHaveBeenNthCalledWith(1, 'openai_account', {});
    expect(invoke).toHaveBeenNthCalledWith(2, 'existing_openai_account', {});
    expect(invoke).toHaveBeenNthCalledWith(3, 'connect_openai', {
      request: { method: 'api_key', api_key: 'sk-secret' },
    });
  });

  it('uses narrow commands for provider settings CRUD', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce({ kind: 'openai', authentication: 'chatgpt' })
      .mockResolvedValueOnce({ kind: 'openai', authentication: 'api_key' })
      .mockResolvedValueOnce(undefined);
    const client = new TauriAgentClient(invoke);

    await client.listProviders();
    await client.createProvider({ kind: 'openai', authentication: 'chatgpt' });
    await client.updateProvider({ kind: 'openai', authentication: 'api_key', api_key: 'sk-secret' });
    await client.deleteProvider('openai');

    expect(invoke).toHaveBeenNthCalledWith(1, 'list_providers', {});
    expect(invoke).toHaveBeenNthCalledWith(2, 'create_provider', {
      provider: { kind: 'openai', authentication: 'chatgpt' },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, 'update_provider', {
      provider: { kind: 'openai', authentication: 'api_key', api_key: 'sk-secret' },
    });
    expect(invoke).toHaveBeenNthCalledWith(4, 'delete_provider', { kind: 'openai' });
  });
});
