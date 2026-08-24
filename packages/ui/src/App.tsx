import { FormEvent, useState } from 'react';

import type { AgentClient, Message } from './contracts';

export interface AppProps {
  client: AgentClient;
}

export function App({ client }: AppProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const prompt = input.trim();
    if (!prompt || pending) {
      return;
    }

    const history = messages;
    setError(null);
    setInput('');
    setPending(true);
    setMessages([...history, { role: 'user', content: prompt }]);

    try {
      const response = await client.respond({ prompt, history });
      setMessages((current) => [...current, response.message]);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Ariadne could not complete the request');
      setMessages(history);
      setInput(prompt);
    } finally {
      setPending(false);
    }
  }

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <p className="eyebrow">Local-first agent</p>
          <h1>Ariadne</h1>
        </div>
        <span className="status"><span aria-hidden="true" /> Ready</span>
      </header>

      <section className="conversation" aria-label="Conversation">
        <div className="messages" role="log" aria-live="polite">
          {messages.length === 0 ? (
            <div className="empty-state">
              <p className="thread-mark" aria-hidden="true">A</p>
              <h2>What should we work through?</h2>
              <p>Ask Ariadne to investigate, plan, or execute a development task.</p>
            </div>
          ) : (
            messages.map((message, index) => (
              <article className={`message message-${message.role}`} key={`${message.role}-${index}`}>
                <p className="message-role">{message.role === 'assistant' ? 'Ariadne' : 'You'}</p>
                <p>{message.content}</p>
              </article>
            ))
          )}
        </div>

        {error ? <p className="request-error" role="alert">{error}</p> : null}
        <form className="composer" onSubmit={submit}>
          <label htmlFor="prompt">Message Ariadne</label>
          <div className="composer-row">
            <textarea
              id="prompt"
              name="prompt"
              value={input}
              onChange={(event) => setInput(event.target.value)}
              placeholder="Describe the task, constraints, and desired outcome…"
              rows={3}
            />
            <button disabled={pending || !input.trim()} type="submit">
              {pending ? 'Working…' : 'Send'}
            </button>
          </div>
        </form>
      </section>
    </main>
  );
}
