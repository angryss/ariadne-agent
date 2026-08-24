import { invoke } from '@tauri-apps/api/core';
import type { AgentClient, RespondRequest, RespondResponse } from '@ariadne/ui';

type Invoker = (command: string, args: Record<string, unknown>) => Promise<unknown>;

export class TauriAgentClient implements AgentClient {
  private readonly invoke: Invoker;

  constructor(invoker: Invoker = (command, args) => invoke(command, args)) {
    this.invoke = invoker;
  }

  async respond(request: RespondRequest): Promise<RespondResponse> {
    const response = await this.invoke('respond', { request });
    if (!isRespondResponse(response)) {
      throw new Error('Ariadne desktop returned an invalid response');
    }
    return response;
  }
}

function isRespondResponse(value: unknown): value is RespondResponse {
  if (!value || typeof value !== 'object' || !('message' in value)) {
    return false;
  }
  const message = value.message;
  return Boolean(
    message &&
      typeof message === 'object' &&
      'role' in message &&
      (message.role === 'assistant' || message.role === 'user') &&
      'content' in message &&
      typeof message.content === 'string',
  );
}
