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
  respond(request: RespondRequest): Promise<RespondResponse>;
  listProfiles?(): Promise<ProfileCatalog>;
}
