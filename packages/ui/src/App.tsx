import { FormEvent, useEffect, useRef, useState } from 'react';

import type {
  AgentClient,
  CompletionDelta,
  ConfiguredProvider,
  Message,
  OpenAiAccount,
  Profile,
  ProviderInput,
} from './contracts';

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

function finalizeResponse(messages: DisplayMessage[], message: Message): DisplayMessage[] {
  const collapsed = messages.map((candidate) =>
    candidate.role === 'thinking' ? { ...candidate, expanded: false } : candidate,
  );
  if (!message.content) {
    return collapsed;
  }

  const last = collapsed.at(-1);
  if (last?.role === 'assistant') {
    return [...collapsed.slice(0, -1), message];
  }
  return [...collapsed, message];
}

export function App({ client }: AppProps) {
  const [messages, setMessages] = useState<DisplayMessage[]>([]);
  const [input, setInput] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState<string | null>(null);
  const [openAiAccount, setOpenAiAccount] = useState<OpenAiAccount | null>(null);
  const [showOpenAi, setShowOpenAi] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [connectingOpenAi, setConnectingOpenAi] = useState(false);
  const [view, setView] = useState<'chat' | 'settings'>('chat');
  const [providerSettings, setProviderSettings] = useState<ConfiguredProvider[]>([]);
  const [editingProvider, setEditingProvider] = useState<ConfiguredProvider['kind'] | null>(null);
  const [providerKind, setProviderKind] = useState<ConfiguredProvider['kind']>('ollama');
  const [ollamaApiBase, setOllamaApiBase] = useState('http://127.0.0.1:11434/v1');
  const [openAiAuthentication, setOpenAiAuthentication] = useState<'api_key' | 'chatgpt'>('chatgpt');
  const [providerApiKey, setProviderApiKey] = useState('');
  const [savingProvider, setSavingProvider] = useState(false);
  const openAiAccountRequest = useRef(0);
  const providerMutationRevision = useRef(0);

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

  useEffect(() => {
    let active = true;
    const request = ++openAiAccountRequest.current;
    if (client.getOpenAiAccount) {
      void client
        .getOpenAiAccount()
        .then((account) => {
          if (active && request === openAiAccountRequest.current) setOpenAiAccount(account);
        })
        .catch(() => {
          if (active && request === openAiAccountRequest.current) {
            setOpenAiAccount({ connected: false, method: null });
          }
        });
    }
    return () => {
      active = false;
    };
  }, [client]);

  useEffect(() => {
    let active = true;
    const revision = providerMutationRevision.current;
    if (client.listProviders) {
      void client
        .listProviders()
        .then((providers) => {
          if (active && revision === providerMutationRevision.current) {
            setProviderSettings(providers);
          }
        })
        .catch((providerError: unknown) => {
          if (active) {
            setError(
              providerError instanceof Error
                ? providerError.message
                : 'Ariadne could not load provider settings',
            );
          }
        });
    }
    return () => {
      active = false;
    };
  }, [client]);

  const activeProfile = profiles.find((profile) => profile.name === selectedProfile);

  async function connectOpenAi(method: 'chatgpt' | 'api_key') {
    if (!client.connectOpenAi || connectingOpenAi) return;
    openAiAccountRequest.current += 1;
    setConnectingOpenAi(true);
    setError(null);
    try {
      const account = await client.connectOpenAi(
        method === 'chatgpt' ? { method } : { method, api_key: apiKey },
      );
      setOpenAiAccount(account);
      setShowOpenAi(false);
    } catch (connectError) {
      setError(
        connectError instanceof Error
          ? connectError.message
          : 'Ariadne could not connect OpenAI',
      );
    } finally {
      setApiKey('');
      setConnectingOpenAi(false);
    }
  }

  function beginAddProvider() {
    const availableKind = providerSettings.some((provider) => provider.kind === 'ollama')
      ? 'openai'
      : 'ollama';
    setEditingProvider(availableKind);
    setProviderKind(availableKind);
    setOllamaApiBase('http://127.0.0.1:11434/v1');
    setOpenAiAuthentication('chatgpt');
    setProviderApiKey('');
  }

  function beginEditProvider(provider: ConfiguredProvider) {
    setEditingProvider(provider.kind);
    setProviderKind(provider.kind);
    if (provider.kind === 'ollama') setOllamaApiBase(provider.api_base);
    else setOpenAiAuthentication(provider.authentication);
    setProviderApiKey('');
  }

  async function saveProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (savingProvider) return;
    const existing = providerSettings.some((provider) => provider.kind === editingProvider);
    const input: ProviderInput =
      providerKind === 'ollama'
        ? { kind: 'ollama', api_base: ollamaApiBase.trim() }
        : openAiAuthentication === 'chatgpt'
          ? { kind: 'openai', authentication: 'chatgpt' }
          : { kind: 'openai', authentication: 'api_key', api_key: providerApiKey };
    const save = existing ? client.updateProvider : client.createProvider;
    if (!save) return;
    setSavingProvider(true);
    setError(null);
    try {
      const saved = await save.call(client, input);
      providerMutationRevision.current += 1;
      setProviderSettings((current) => [
        ...current.filter((provider) => provider.kind !== saved.kind),
        saved,
      ]);
      setEditingProvider(null);
    } catch (providerError) {
      setError(
        providerError instanceof Error
          ? providerError.message
          : 'Ariadne could not save the provider',
      );
    } finally {
      setProviderApiKey('');
      setSavingProvider(false);
    }
  }

  async function removeProvider(kind: ConfiguredProvider['kind']) {
    if (!client.deleteProvider || savingProvider) return;
    setSavingProvider(true);
    setError(null);
    try {
      await client.deleteProvider(kind);
      providerMutationRevision.current += 1;
      setProviderSettings((current) => current.filter((provider) => provider.kind !== kind));
      setEditingProvider(null);
    } catch (providerError) {
      setError(
        providerError instanceof Error
          ? providerError.message
          : 'Ariadne could not delete the provider',
      );
    } finally {
      setSavingProvider(false);
    }
  }

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
      const response = await client.respond({
        ...(selectedProfile ? { profile: selectedProfile } : {}),
        prompt,
        history,
      }, (delta) => {
        setMessages((current) => appendDelta(current, delta));
      });
      setMessages((current) => finalizeResponse(current, response.message));
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
          {client.connectOpenAi ? (
            <button
              className="account-button"
              onClick={() => {
                if (showOpenAi) setApiKey('');
                setShowOpenAi(!showOpenAi);
              }}
              type="button"
            >
              {openAiAccount?.connected
                ? openAiAccount.method === 'chatgpt'
                  ? `Connected with ChatGPT${openAiAccount.plan ? ` ${formatPlan(openAiAccount.plan)}` : ''}`
                  : 'Connected with API key'
                : 'Connect OpenAI'}
            </button>
          ) : null}
          {client.listProviders ? (
            <button
              className="account-button"
              onClick={() => {
                setView(view === 'settings' ? 'chat' : 'settings');
                setEditingProvider(null);
                setError(null);
              }}
              type="button"
            >
              {view === 'settings' ? 'Back to chat' : 'Settings'}
            </button>
          ) : null}
          <span className="status"><span aria-hidden="true" /> Ready</span>
        </div>
      </header>

      {view === 'chat' && showOpenAi && client.connectOpenAi ? (
        <section className="account-panel" aria-label="Connect OpenAI">
          <button
            disabled={connectingOpenAi}
            onClick={() => void connectOpenAi('chatgpt')}
            type="button"
          >
            {connectingOpenAi ? 'Connecting…' : 'Use ChatGPT subscription'}
          </button>
          <span>or</span>
          <label htmlFor="openai-api-key">OpenAI API key</label>
          <input
            autoComplete="off"
            id="openai-api-key"
            onChange={(event) => setApiKey(event.target.value)}
            type="password"
            value={apiKey}
          />
          <button
            disabled={connectingOpenAi || !apiKey.trim()}
            onClick={() => void connectOpenAi('api_key')}
            type="button"
          >
            Save API key
          </button>
        </section>
      ) : null}

      {view === 'chat' && activeProfile ? (
        <aside className="profile-summary" aria-label="Active profile">
          <strong>{activeProfile.model}</strong>
          <span>{activeProfile.provider}</span>
          {activeProfile.active_skills.map((skill) => (
            <span key={`skill-${skill}`}>{skill} skill</span>
          ))}
          {activeProfile.mcp_servers.map((server) => (
            <span key={`mcp-${server}`}>{server} MCP</span>
          ))}
          {activeProfile.capabilities.map((capability) => (
            <span key={`capability-${capability}`}>{capability} capability</span>
          ))}
        </aside>
      ) : null}

      {view === 'settings' ? (
        <section className="settings-page" aria-label="Settings">
          <div className="settings-heading">
            <div>
              <p className="eyebrow">Settings</p>
              <h2>Providers</h2>
            </div>
            <button disabled={providerSettings.length >= 2} onClick={beginAddProvider} type="button">
              Add provider
            </button>
          </div>
          {providerSettings.length === 0 ? (
            <p className="settings-empty">No providers configured.</p>
          ) : (
            <div className="provider-list">
              {providerSettings.map((provider) => (
                <article className="provider-card" key={provider.kind}>
                  <div>
                    <h3>{providerTitle(provider.kind)}</h3>
                    <p>
                      {provider.kind === 'ollama'
                        ? provider.api_base
                        : provider.authentication === 'chatgpt'
                          ? 'ChatGPT subscription'
                          : 'API key'}
                    </p>
                  </div>
                  <div className="provider-actions">
                    <button
                      aria-label={`Edit ${providerTitle(provider.kind)}`}
                      onClick={() => beginEditProvider(provider)}
                      type="button"
                    >
                      Edit
                    </button>
                    <button
                      aria-label={`Delete ${providerTitle(provider.kind)}`}
                      onClick={() => void removeProvider(provider.kind)}
                      type="button"
                    >
                      Delete
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
          {editingProvider ? (
            <form className="provider-form" onSubmit={(event) => void saveProvider(event)}>
              <h3>
                {providerSettings.some((provider) => provider.kind === editingProvider)
                  ? 'Edit provider'
                  : 'Add provider'}
              </h3>
              <label htmlFor="provider-type">Provider type</label>
              <select
                disabled={providerSettings.some((provider) => provider.kind === editingProvider)}
                id="provider-type"
                onChange={(event) => {
                  const kind = event.target.value as ConfiguredProvider['kind'];
                  setProviderKind(kind);
                  setEditingProvider(kind);
                }}
                value={providerKind}
              >
                <option
                  disabled={providerSettings.some((provider) => provider.kind === 'ollama')}
                  value="ollama"
                >
                  Ollama
                </option>
                <option
                  disabled={providerSettings.some((provider) => provider.kind === 'openai')}
                  value="openai"
                >
                  OpenAI
                </option>
              </select>
              {providerKind === 'ollama' ? (
                <>
                  <label htmlFor="ollama-api-base">Ollama API base URL</label>
                  <input
                    id="ollama-api-base"
                    onChange={(event) => setOllamaApiBase(event.target.value)}
                    required
                    type="url"
                    value={ollamaApiBase}
                  />
                </>
              ) : (
                <>
                  <label htmlFor="openai-authentication">OpenAI authentication</label>
                  <select
                    id="openai-authentication"
                    onChange={(event) =>
                      setOpenAiAuthentication(event.target.value as 'api_key' | 'chatgpt')
                    }
                    value={openAiAuthentication}
                  >
                    <option value="chatgpt">ChatGPT subscription</option>
                    <option value="api_key">API key</option>
                  </select>
                  {openAiAuthentication === 'api_key' ? (
                    <>
                      <label htmlFor="provider-openai-api-key">OpenAI API key</label>
                      <input
                        autoComplete="off"
                        id="provider-openai-api-key"
                        onChange={(event) => setProviderApiKey(event.target.value)}
                        required
                        type="password"
                        value={providerApiKey}
                      />
                    </>
                  ) : (
                    <p>A browser window will open so you can sign in to ChatGPT.</p>
                  )}
                </>
              )}
              <div className="provider-actions">
                <button disabled={savingProvider} type="submit">Save provider</button>
                <button
                  onClick={() => {
                    setEditingProvider(null);
                    setProviderApiKey('');
                  }}
                  type="button"
                >
                  Cancel
                </button>
              </div>
            </form>
          ) : null}
          {error ? <p className="request-error" role="alert">{error}</p> : null}
        </section>
      ) : null}

      {view === 'chat' ? <section className="conversation" aria-label="Conversation">
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
      </section> : null}
    </main>
  );
}

function formatPlan(plan: string): string {
  return plan.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function providerTitle(kind: ConfiguredProvider['kind']): string {
  return kind === 'ollama' ? 'Ollama' : 'OpenAI';
}
