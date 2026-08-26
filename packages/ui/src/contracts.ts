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
}

export interface ProfileCatalog {
  default_profile: string;
  profiles: Profile[];
}

export interface AgentClient {
  respond(request: RespondRequest, onDelta?: CompletionDeltaHandler): Promise<RespondResponse>;
  listProfiles?(): Promise<ProfileCatalog>;
}
