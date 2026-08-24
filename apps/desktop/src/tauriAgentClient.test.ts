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
});
