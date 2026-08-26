import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AgentClient,
  CompletionDelta,
  CompletionDeltaHandler,
  Profile,
  ProfileCatalog,
  RespondRequest,
  RespondResponse,
} from '@ariadne/ui';

type Invoker = (command: string, args: Record<string, unknown>) => Promise<unknown>;
interface DeltaChannel {
  onmessage: ((message: CompletionDelta) => void) | null;
}
type ChannelFactory = () => DeltaChannel;

export class TauriAgentClient implements AgentClient {
  private readonly invoke: Invoker;
  private readonly createChannel: ChannelFactory;

  constructor(
    invoker: Invoker = (command, args) => invoke(command, args),
    createChannel: ChannelFactory = () => new Channel<CompletionDelta>(),
  ) {
    this.invoke = invoker;
    this.createChannel = createChannel;
  }

  async listProfiles(): Promise<ProfileCatalog> {
    const profiles = await this.invoke('profiles', {});
    if (!isProfileCatalog(profiles)) {
      throw new Error('Ariadne desktop returned invalid profile data');
    }
    return profiles;
  }

  async respond(
    request: RespondRequest,
    onDelta?: CompletionDeltaHandler,
  ): Promise<RespondResponse> {
    let command = 'respond';
    let args: Record<string, unknown> = { request };
    let invalidDelta = false;
    if (onDelta) {
      command = 'respond_stream';
      const onEvent = this.createChannel();
      onEvent.onmessage = (message) => {
        if (isCompletionDelta(message)) {
          onDelta(message);
        } else {
          invalidDelta = true;
        }
      };
      args = { request, onEvent };
    }

    const response = await this.invoke(command, args);
    if (invalidDelta) {
      throw new Error('Ariadne desktop returned invalid stream data');
    }
    if (!isRespondResponse(response)) {
      throw new Error('Ariadne desktop returned an invalid response');
    }
    return response;
  }
}

function isCompletionDelta(value: unknown): value is CompletionDelta {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'kind' in value &&
      (value.kind === 'thinking' || value.kind === 'content') &&
      'content' in value &&
      typeof value.content === 'string',
  );
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
