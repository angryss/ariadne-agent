export type MessageRole = 'user' | 'assistant';

export interface Message {
  role: MessageRole;
  content: string;
}

export interface RespondRequest {
  profile?: string;
  prompt: string;
  history: Message[];
}

export interface RespondResponse {
  message: Message;
}

export type CompletionDelta =
  | { kind: 'thinking'; content: string }
  | { kind: 'content'; content: string };

export type CompletionDeltaHandler = (delta: CompletionDelta) => void;

export interface Profile {
  name: string;
  provider: string;
  model: string;
  active_skills: string[];
  mcp_servers: string[];
  capabilities: string[];
}

export interface ProfileCatalog {
  default_profile: string;
  profiles: Profile[];
}

export type OpenAiAccount =
  | { connected: false; method: null }
  | { connected: true; method: 'api_key' }
  | { connected: true; method: 'chatgpt'; plan?: string };

export type ConnectOpenAiRequest =
  | { method: 'chatgpt' }
  | { method: 'api_key'; api_key: string };

export type ConfiguredProvider =
  | { kind: 'ollama'; api_base: string }
  | { kind: 'openai'; authentication: 'api_key' | 'chatgpt' };

export type ProviderInput =
  | { kind: 'ollama'; api_base: string }
  | { kind: 'openai'; authentication: 'chatgpt' }
  | { kind: 'openai'; authentication: 'api_key'; api_key: string };

export interface AgentClient {
  respond(request: RespondRequest, onDelta?: CompletionDeltaHandler): Promise<RespondResponse>;
  listProfiles?(): Promise<ProfileCatalog>;
  getOpenAiAccount?(): Promise<OpenAiAccount>;
  connectOpenAi?(request: ConnectOpenAiRequest): Promise<OpenAiAccount>;
  listProviders?(): Promise<ConfiguredProvider[]>;
  createProvider?(provider: ProviderInput): Promise<ConfiguredProvider>;
  updateProvider?(provider: ProviderInput): Promise<ConfiguredProvider>;
  deleteProvider?(kind: ConfiguredProvider['kind']): Promise<void>;
}
