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
      profiles: [
        {
          name: 'local',
          provider: 'ollama',
          model: 'qwen3:8b',
          active_skills: [],
          mcp_servers: ['filesystem'],
        },
      ],
    });
    const client = new TauriAgentClient(invoke);

    const profiles = await client.listProfiles();

    expect(invoke).toHaveBeenCalledWith('profiles', {});
    expect(profiles.default_profile).toBe('local');
    expect(profiles.profiles[0]!.mcp_servers).toEqual(['filesystem']);
  });
});
