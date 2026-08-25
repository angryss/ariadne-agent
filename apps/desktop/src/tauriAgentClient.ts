import { invoke } from '@tauri-apps/api/core';
import type {
  AgentClient,
  Profile,
  ProfileCatalog,
  RespondRequest,
  RespondResponse,
} from '@ariadne/ui';

type Invoker = (command: string, args: Record<string, unknown>) => Promise<unknown>;

export class TauriAgentClient implements AgentClient {
  private readonly invoke: Invoker;

  constructor(invoker: Invoker = (command, args) => invoke(command, args)) {
    this.invoke = invoker;
  }

  async listProfiles(): Promise<ProfileCatalog> {
    const profiles = await this.invoke('profiles', {});
    if (!isProfileCatalog(profiles)) {
      throw new Error('Ariadne desktop returned invalid profile data');
    }
    return profiles;
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

function isProfileCatalog(value: unknown): value is ProfileCatalog {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'default_profile' in value &&
      typeof value.default_profile === 'string' &&
      'profiles' in value &&
      Array.isArray(value.profiles) &&
      value.profiles.every(isProfile),
  );
}

function isProfile(value: unknown): value is Profile {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'name' in value &&
      typeof value.name === 'string' &&
      'provider' in value &&
      typeof value.provider === 'string' &&
      'model' in value &&
      typeof value.model === 'string' &&
      'active_skills' in value &&
      Array.isArray(value.active_skills) &&
      value.active_skills.every((skill) => typeof skill === 'string') &&
      'mcp_servers' in value &&
      Array.isArray(value.mcp_servers) &&
      value.mcp_servers.every((server) => typeof server === 'string'),
  );
}
