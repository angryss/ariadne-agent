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
