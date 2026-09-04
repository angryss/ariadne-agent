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

export interface ProfileProvider {
  provider: string;
  model: string;
  enabled?: boolean;
  default?: boolean;
}

export interface Profile {
  name: string;
  providers: ProfileProvider[];
  active_skills: string[];
  mcp_servers: string[];
  capabilities: string[];
}

export interface ProfileCatalog {
  default_profile: string;
  provider_ids: string[];
  profiles: Profile[];
  configured_profiles: Profile[];
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
  | { kind: 'openrouter' }
  | { kind: 'openai'; authentication: 'api_key' | 'chatgpt'; reuse_existing?: boolean }
  | { kind: 'anthropic'; authentication: 'api_key' | 'subscription' };

export type ProviderInput =
  | { kind: 'ollama'; api_base: string }
  | { kind: 'openrouter' }
  | { kind: 'openai'; authentication: 'chatgpt'; reuse_existing?: boolean }
  | { kind: 'openai'; authentication: 'api_key'; api_key: string }
  | { kind: 'anthropic'; authentication: 'api_key' | 'subscription' };

export interface AgentClient {
  respond(request: RespondRequest, onDelta?: CompletionDeltaHandler): Promise<RespondResponse>;
  listProfiles?(): Promise<ProfileCatalog>;
  createProfile?(profile: Profile): Promise<Profile>;
  updateProfile?(name: string, profile: Profile): Promise<Profile>;
  deleteProfile?(name: string): Promise<void>;
  getOpenAiAccount?(): Promise<OpenAiAccount>;
  getExistingOpenAiAccount?(): Promise<OpenAiAccount>;
  connectOpenAi?(request: ConnectOpenAiRequest): Promise<OpenAiAccount>;
  listProviders?(profile: string): Promise<ConfiguredProvider[]>;
  createProvider?(provider: ProviderInput, profile: string): Promise<ConfiguredProvider>;
  updateProvider?(provider: ProviderInput, profile: string): Promise<ConfiguredProvider>;
  deleteProvider?(kind: ConfiguredProvider['kind'], profile: string): Promise<void>;
}
