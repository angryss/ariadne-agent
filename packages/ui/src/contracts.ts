export type MessageRole = 'user' | 'assistant';

export interface Message {
  role: MessageRole;
  content: string;
}

export interface RespondRequest {
  prompt: string;
  history: Message[];
}

export interface RespondResponse {
  message: Message;
}

export interface AgentClient {
  respond(request: RespondRequest): Promise<RespondResponse>;
}
