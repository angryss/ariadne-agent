import { FormEvent, useEffect, useState } from 'react';

import type { AgentClient, CompletionDelta, Message, Profile } from './contracts';

export interface AppProps {
  client: AgentClient;
}

interface ThinkingMessage {
  role: 'thinking';
  content: string;
  expanded: boolean;
}

type DisplayMessage = Message | ThinkingMessage;

function conversationHistory(messages: DisplayMessage[]): Message[] {
  return messages.filter((message): message is Message => message.role !== 'thinking');
}

function appendDelta(messages: DisplayMessage[], delta: CompletionDelta): DisplayMessage[] {
  if (!delta.content) {
    return messages;
  }
  if (delta.kind === 'thinking') {
    const last = messages.at(-1);
    if (last?.role === 'thinking') {
      return [
        ...messages.slice(0, -1),
        { ...last, content: last.content + delta.content, expanded: true },
      ];
    }
    return [...messages, { role: 'thinking', content: delta.content, expanded: true }];
  }

  const collapsed = messages.map((message) =>
    message.role === 'thinking' ? { ...message, expanded: false } : message,
  );
  const last = collapsed.at(-1);
  if (last?.role === 'assistant') {
    return [
      ...collapsed.slice(0, -1),
      { ...last, content: last.content + delta.content },
    ];
  }
  return [...collapsed, { role: 'assistant', content: delta.content }];
}

export function App({ client }: AppProps) {
  const [messages, setMessages] = useState<DisplayMessage[]>([]);
  const [input, setInput] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    if (!client.listProfiles) {
      return () => {
        active = false;
      };
    }

    void client
      .listProfiles()
      .then((catalog) => {
        if (!active) {
          return;
        }
        setProfiles(catalog.profiles);
        setSelectedProfile(catalog.default_profile);
      })
      .catch((profileError: unknown) => {
        if (active) {
          setError(
            profileError instanceof Error
              ? profileError.message
              : 'Ariadne could not load profiles',
          );
        }
      });

    return () => {
      active = false;
    };
  }, [client]);

  const activeProfile = profiles.find((profile) => profile.name === selectedProfile);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const prompt = input.trim();
    if (!prompt || pending) {
      return;
    }

    const displayHistory = messages;
    const history = conversationHistory(displayHistory);
    setError(null);
    setInput('');
    setPending(true);
    setMessages([...displayHistory, { role: 'user', content: prompt }]);

    try {
      let receivedContent = false;
      const response = await client.respond({
        ...(selectedProfile ? { profile: selectedProfile } : {}),
        prompt,
        history,
      }, (delta) => {
        receivedContent ||= delta.kind === 'content' && delta.content.length > 0;
        setMessages((current) => appendDelta(current, delta));
      });
      if (!receivedContent) {
        setMessages((current) => [...current, response.message]);
      }
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Ariadne could not complete the request');
      setMessages(displayHistory);
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
        <div className="header-actions">
          {profiles.length > 0 ? (
            <label className="profile-picker" htmlFor="profile">
              <span>Profile</span>
              <select
                disabled={pending}
                id="profile"
                onChange={(event) => {
                  setSelectedProfile(event.target.value);
                  setMessages([]);
                  setError(null);
                }}
                value={selectedProfile ?? ''}
              >
                {profiles.map((profile) => (
                  <option key={profile.name} value={profile.name}>
                    {profile.name}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          <span className="status"><span aria-hidden="true" /> Ready</span>
        </div>
      </header>

      {activeProfile ? (
        <aside className="profile-summary" aria-label="Active profile">
          <strong>{activeProfile.model}</strong>
          <span>{activeProfile.provider}</span>
          {activeProfile.active_skills.map((skill) => (
            <span key={`skill-${skill}`}>{skill} skill</span>
          ))}
          {activeProfile.mcp_servers.map((server) => (
            <span key={`mcp-${server}`}>{server} MCP</span>
          ))}
        </aside>
      ) : null}

      <section className="conversation" aria-label="Conversation">
        <div className="messages" role="log" aria-live="polite">
          {messages.length === 0 ? (
            <div className="empty-state">
              <p className="thread-mark" aria-hidden="true">A</p>
              <h2>What should we work through?</h2>
              <p>Ask Ariadne to investigate, plan, or execute a development task.</p>
            </div>
          ) : (
            messages.map((message, index) =>
              message.role === 'thinking' ? (
                <details
                  className="thinking-block"
                  key={`thinking-${index}`}
                  open={message.expanded}
                  onToggle={(event) => {
                    const expanded = event.currentTarget.open;
                    setMessages((current) =>
                      current.map((candidate, candidateIndex) =>
                        candidateIndex === index && candidate.role === 'thinking'
                          ? { ...candidate, expanded }
                          : candidate,
                      ),
                    );
                  }}
                >
                  <summary>Thinking</summary>
                  <p>{message.content}</p>
                </details>
              ) : (
                <article className={`message message-${message.role}`} key={`${message.role}-${index}`}>
                  <p className="message-role">{message.role === 'assistant' ? 'Ariadne' : 'You'}</p>
                  <p>{message.content}</p>
                </article>
              ),
            )
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
              onKeyDown={(event) => {
                if (
                  event.key === 'Enter' &&
                  !event.shiftKey &&
                  !event.altKey &&
                  !event.ctrlKey &&
                  !event.metaKey &&
                  !event.nativeEvent.isComposing
                ) {
                  event.preventDefault();
                  event.currentTarget.form?.requestSubmit();
                }
              }}
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
