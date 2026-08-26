import type {
  AgentClient,
  CompletionDelta,
  CompletionDeltaHandler,
  Profile,
  ProfileCatalog,
  RespondRequest,
  RespondResponse,
} from '@ariadne/ui';

type Fetcher = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export class HttpAgentClient implements AgentClient {
  private readonly endpoint: string;
  private readonly profilesEndpoint: string;
  private readonly fetcher: Fetcher;

  constructor(
    endpoint = defaultEndpoint(),
    fetcher: Fetcher = globalThis.fetch.bind(globalThis),
    profilesEndpoint = endpoint.replace(/\/respond$/, '/profiles'),
  ) {
    this.endpoint = endpoint;
    this.profilesEndpoint = profilesEndpoint;
    this.fetcher = fetcher;
  }

  async listProfiles(): Promise<ProfileCatalog> {
    const response = await this.fetcher(this.profilesEndpoint, {
      method: 'GET',
      headers: { accept: 'application/json' },
    });
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new Error(
        response.ok
          ? 'Ariadne API returned invalid profile data'
          : `Ariadne API returned ${response.status}`,
      );
    }
    if (!response.ok) {
      throw new Error(readApiError(body) ?? `Ariadne API returned ${response.status}`);
    }
    if (!isProfileCatalog(body)) {
      throw new Error('Ariadne API returned invalid profile data');
    }
    return body;
  }

  async respond(
    request: RespondRequest,
    onDelta?: CompletionDeltaHandler,
  ): Promise<RespondResponse> {
    if (onDelta) {
      return this.respondStream(request, onDelta);
    }
    const response = await this.fetcher(this.endpoint, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
    });
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new Error(
        response.ok ? 'Ariadne API returned an invalid response' : `Ariadne API returned ${response.status}`,
      );
    }

    if (!response.ok) {
      throw new Error(readApiError(body) ?? `Ariadne API returned ${response.status}`);
    }
    if (!isRespondResponse(body)) {
      throw new Error('Ariadne API returned an invalid response');
    }

    return body;
  }

  private async respondStream(
    request: RespondRequest,
    onDelta: CompletionDeltaHandler,
  ): Promise<RespondResponse> {
    const response = await this.fetcher(`${this.endpoint}/stream`, {
      method: 'POST',
      headers: {
        accept: 'text/event-stream',
        'content-type': 'application/json',
      },
      body: JSON.stringify(request),
    });
    if (!response.ok) {
      let body: unknown;
      try {
        body = await response.json();
      } catch {
        throw new Error(`Ariadne API returned ${response.status}`);
      }
      throw new Error(readApiError(body) ?? `Ariadne API returned ${response.status}`);
    }
    if (!response.body) {
      throw new Error('Ariadne API returned an invalid stream');
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let pending = '';
    let result: RespondResponse | null = null;

    const processRecords = (complete: boolean) => {
      pending = pending.replaceAll('\r\n', '\n');
      const records = pending.split('\n\n');
      pending = complete ? '' : (records.pop() ?? '');
      for (const record of records) {
        const data = record
          .split('\n')
          .filter((line) => line.startsWith('data:'))
          .map((line) => line.slice(5).trimStart())
          .join('\n');
        if (!data) {
          continue;
        }
        let event: unknown;
        try {
          event = JSON.parse(data);
        } catch {
          throw new Error('Ariadne API returned an invalid stream event');
        }
        if (isCompletionDelta(event)) {
          onDelta(event);
        } else if (isDoneEvent(event)) {
          result = { message: event.message };
        } else if (isErrorEvent(event)) {
          throw new Error(event.message);
        } else {
          throw new Error('Ariadne API returned an invalid stream event');
        }
      }
    };

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        pending += decoder.decode();
        if (pending) {
          pending += '\n\n';
        }
        processRecords(true);
        break;
      }
      pending += decoder.decode(value, { stream: true });
      processRecords(false);
    }

    if (!result) {
      throw new Error('Ariadne API stream ended without a response');
    }
    return result;
  }
}

function defaultEndpoint(): string {
  const baseUrl = import.meta.env.VITE_ARIADNE_API_URL?.replace(/\/$/, '') ?? '';
  return `${baseUrl}/v1/respond`;
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

function isDoneEvent(value: unknown): value is { kind: 'done'; message: RespondResponse['message'] } {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'kind' in value &&
      value.kind === 'done' &&
      'message' in value &&
      isMessage(value.message),
  );
}

function isErrorEvent(value: unknown): value is { kind: 'error'; message: string } {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'kind' in value &&
      value.kind === 'error' &&
      'message' in value &&
      typeof value.message === 'string',
  );
}

function isMessage(value: unknown): value is RespondResponse['message'] {
  return Boolean(
    value &&
      typeof value === 'object' &&
      'role' in value &&
      (value.role === 'assistant' || value.role === 'user') &&
      'content' in value &&
      typeof value.content === 'string',
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
      value.mcp_servers.every((server) => typeof server === 'string') &&
      'capabilities' in value &&
      Array.isArray(value.capabilities) &&
      value.capabilities.every((capability) => typeof capability === 'string'),
  );
}

function readApiError(value: unknown): string | null {
  if (!value || typeof value !== 'object' || !('error' in value)) {
    return null;
  }
  const error = value.error;
  if (!error || typeof error !== 'object' || !('message' in error)) {
    return null;
  }
  return typeof error.message === 'string' ? error.message : null;
}
