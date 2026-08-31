import { FormEvent, useEffect, useRef, useState } from 'react';

import { ThemeToggle } from './components/theme-toggle';
import { Typeahead } from './components/typeahead';
import { Badge } from './components/ui/badge';
import { Button } from './components/ui/button';
import { Input } from './components/ui/input';
import { Textarea } from './components/ui/textarea';
import type {
  AgentClient,
  CompletionDelta,
  ConfiguredProvider,
  Message,
  OpenAiAccount,
  Profile,
  ProfileProvider,
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

const PROVIDER_KINDS: readonly ConfiguredProvider['kind'][] = [
  'anthropic',
  'ollama',
  'openai',
];

function sortedProfiles(profiles: Profile[]): Profile[] {
  return [...profiles].sort((left, right) => left.name.localeCompare(right.name));
}

function profileProviderOptions(providerIds: readonly string[]): string[] {
  return [...new Set(providerIds)];
}

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
  const [profileProviderIds, setProfileProviderIds] = useState<string[]>([]);
  const [selectedProfile, setSelectedProfile] = useState<string | null>(null);
  const [openAiAccount, setOpenAiAccount] = useState<OpenAiAccount | null>(null);
  const [existingOpenAiAccount, setExistingOpenAiAccount] = useState<OpenAiAccount | null>(null);
  const [discoveringExistingOpenAiAccount, setDiscoveringExistingOpenAiAccount] = useState(
    Boolean(client.getExistingOpenAiAccount),
  );
  const [showOpenAi, setShowOpenAi] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [connectingOpenAi, setConnectingOpenAi] = useState(false);
  const [view, setView] = useState<'chat' | 'settings'>('chat');
  const [providerSettings, setProviderSettings] = useState<ConfiguredProvider[]>([]);
  const [editingProvider, setEditingProvider] = useState<ConfiguredProvider['kind'] | null>(null);
  const [providerKind, setProviderKind] = useState<ConfiguredProvider['kind']>('ollama');
  const [providerTypeQuery, setProviderTypeQuery] = useState('Ollama');
  const [providerTypeOpen, setProviderTypeOpen] = useState(false);
  const [providerTypeDirty, setProviderTypeDirty] = useState(false);
  const [activeProviderKind, setActiveProviderKind] = useState<ConfiguredProvider['kind'] | null>('ollama');
  const [ollamaApiBase, setOllamaApiBase] = useState('http://127.0.0.1:11434/v1');
  const [openAiAuthentication, setOpenAiAuthentication] = useState<'api_key' | 'chatgpt'>('chatgpt');
  const [anthropicAuthentication, setAnthropicAuthentication] = useState<'api_key' | 'subscription'>('subscription');
  const [reuseExistingChatgpt, setReuseExistingChatgpt] = useState<boolean | null>(null);
  const [providerApiKey, setProviderApiKey] = useState('');
  const [savingProvider, setSavingProvider] = useState(false);
  const [settingsSection, setSettingsSection] = useState<'profiles' | 'provider-credentials'>(
    client.createProfile || client.updateProfile || client.deleteProfile
      ? 'profiles'
      : 'provider-credentials',
  );
  const [addingProfile, setAddingProfile] = useState(false);
  const [savingProfile, setSavingProfile] = useState(false);
  const [profileName, setProfileName] = useState('');
  const [profileProviders, setProfileProviders] = useState<ProfileProvider[]>([]);
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
        setProfileProviderIds(catalog.provider_ids);
        setSelectedProfile(catalog.default_profile);
      })
      .catch((profileError: unknown) => {
        if (active) {
          setError(
            profileError instanceof Error
              ? profileError.message
              : 'Rynna could not load profiles',
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
    if (client.getExistingOpenAiAccount) {
      setDiscoveringExistingOpenAiAccount(true);
      void client
        .getExistingOpenAiAccount()
        .then((account) => {
          if (active) setExistingOpenAiAccount(account);
        })
        .catch(() => {
          if (active) setExistingOpenAiAccount({ connected: false, method: null });
        })
        .finally(() => {
          if (active) setDiscoveringExistingOpenAiAccount(false);
        });
    } else {
      setDiscoveringExistingOpenAiAccount(false);
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
                : 'Rynna could not load provider settings',
            );
          }
        });
    }
    return () => {
      active = false;
    };
  }, [client]);

  const activeProfile = profiles.find((profile) => profile.name === selectedProfile);
  const canEditProfiles = Boolean(client.createProfile || client.updateProfile || client.deleteProfile);
  const canOpenSettings = Boolean(client.listProviders || canEditProfiles);

  useEffect(() => {
    if (addingProfile) {
      return;
    }
    if (!activeProfile) {
      setProfileName('');
      setProfileProviders([]);
      return;
    }
    setProfileName(activeProfile.name);
    setProfileProviders(activeProfile.providers.map((provider) => ({ ...provider })));
  }, [activeProfile, addingProfile]);

  function selectProfile(name: string) {
    setSelectedProfile(name);
    setAddingProfile(false);
    setMessages([]);
    setError(null);
  }

  function beginAddProfile() {
    setAddingProfile(true);
    setProfileName('');
    setProfileProviders([
      activeProfile?.providers[0]
        ? { ...activeProfile.providers[0] }
        : { provider: 'ollama', model: 'qwen3:8b' },
    ]);
    setError(null);
  }

  function updateProfileProvider(
    index: number,
    field: keyof ProfileProvider,
    value: string,
  ) {
    setProfileProviders((current) =>
      current.map((provider, providerIndex) =>
        providerIndex === index ? { ...provider, [field]: value } : provider,
      ),
    );
  }

  function addProfileProvider() {
    setProfileProviders((current) => [...current, { provider: '', model: '' }]);
  }

  function removeProfileProvider(index: number) {
    setProfileProviders((current) => current.filter((_, providerIndex) => providerIndex !== index));
  }

  function profileDraft(): Profile {
    return {
      name: profileName.trim(),
      providers: profileProviders.map((provider) => ({
        provider: provider.provider.trim(),
        model: provider.model.trim(),
      })),
      active_skills: addingProfile ? [] : (activeProfile?.active_skills ?? []),
      mcp_servers: addingProfile ? [] : (activeProfile?.mcp_servers ?? []),
      capabilities: addingProfile ? [] : (activeProfile?.capabilities ?? []),
    };
  }

  async function saveProfile(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (savingProfile) return;
    const draft = profileDraft();
    if (
      !draft.name ||
      draft.providers.length === 0 ||
      draft.providers.some((provider) => !provider.provider || !provider.model)
    ) return;
    const save = addingProfile ? client.createProfile : client.updateProfile;
    if (!save) return;
    setSavingProfile(true);
    setError(null);
    try {
      const saved = addingProfile
        ? await client.createProfile!(draft)
        : await client.updateProfile!(selectedProfile ?? draft.name, draft);
      setProfiles((current) => {
        const withoutPrevious = addingProfile
          ? current
          : current.filter((profile) => profile.name !== selectedProfile);
        return [...withoutPrevious.filter((profile) => profile.name !== saved.name), saved];
      });
      setSelectedProfile(saved.name);
      setAddingProfile(false);
      setProfileName(saved.name);
      setProfileProviders(saved.providers.map((provider) => ({ ...provider })));
    } catch (profileError) {
      setError(
        profileError instanceof Error ? profileError.message : 'Rynna could not save the profile',
      );
    } finally {
      setSavingProfile(false);
    }
  }

  async function removeProfile() {
    if (!client.deleteProfile || savingProfile || addingProfile || !selectedProfile) return;
    if (profiles.length <= 1) return;
    setSavingProfile(true);
    setError(null);
    try {
      await client.deleteProfile(selectedProfile);
      const remaining = profiles.filter((profile) => profile.name !== selectedProfile);
      setProfiles(remaining);
      const next = remaining[0]?.name ?? null;
      setSelectedProfile(next);
      setMessages([]);
    } catch (profileError) {
      setError(
        profileError instanceof Error ? profileError.message : 'Rynna could not delete the profile',
      );
    } finally {
      setSavingProfile(false);
    }
  }

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
          : 'Rynna could not connect OpenAI',
      );
    } finally {
      setApiKey('');
      setConnectingOpenAi(false);
    }
  }

  function matchingProviderKinds(query: string, showAll = !providerTypeDirty) {
    return PROVIDER_KINDS.filter(
      (kind) =>
        !providerSettings.some((provider) => provider.kind === kind) &&
        (showAll || providerTitle(kind).toLowerCase().includes(query.toLowerCase())),
    );
  }

  function beginAddProvider() {
    const availableKind = (['ollama', 'openai', 'anthropic'] as const).find(
      (kind) => !providerSettings.some((provider) => provider.kind === kind),
    ) ?? 'ollama';
    setEditingProvider(availableKind);
    setProviderKind(availableKind);
    setProviderTypeQuery(providerTitle(availableKind));
    setProviderTypeOpen(false);
    setProviderTypeDirty(false);
    setActiveProviderKind(availableKind);
    setOllamaApiBase('http://127.0.0.1:11434/v1');
    setOpenAiAuthentication('chatgpt');
    setAnthropicAuthentication('subscription');
    setReuseExistingChatgpt(null);
    setProviderApiKey('');
  }

  function beginEditProvider(provider: ConfiguredProvider) {
    setEditingProvider(provider.kind);
    setProviderKind(provider.kind);
    setProviderTypeQuery(providerTitle(provider.kind));
    setProviderTypeOpen(false);
    setProviderTypeDirty(false);
    setActiveProviderKind(provider.kind);
    if (provider.kind === 'ollama') setOllamaApiBase(provider.api_base);
    else if (provider.kind === 'openai') setOpenAiAuthentication(provider.authentication);
    else setAnthropicAuthentication(provider.authentication);
    setReuseExistingChatgpt(null);
    setProviderApiKey('');
  }

  function selectProviderKind(kind: ConfiguredProvider['kind']) {
    setProviderKind(kind);
    setEditingProvider(kind);
    setProviderTypeQuery(providerTitle(kind));
    setProviderTypeOpen(false);
    setProviderTypeDirty(false);
    setActiveProviderKind(kind);
  }

  async function refreshOpenAiAccountStatus() {
    if (!client.getOpenAiAccount) return;
    const request = ++openAiAccountRequest.current;
    try {
      const account = await client.getOpenAiAccount();
      if (request === openAiAccountRequest.current) setOpenAiAccount(account);
    } catch {
      if (request === openAiAccountRequest.current) {
        setOpenAiAccount({ connected: false, method: null });
      }
    }
  }

  async function saveProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (savingProvider) return;
    const existing = providerSettings.some((provider) => provider.kind === editingProvider);
    const input: ProviderInput =
      providerKind === 'ollama'
        ? { kind: 'ollama', api_base: ollamaApiBase.trim() }
        : providerKind === 'anthropic'
          ? { kind: 'anthropic', authentication: anthropicAuthentication }
          : openAiAuthentication === 'chatgpt'
          ? {
              kind: 'openai',
              authentication: 'chatgpt',
              ...(reuseExistingChatgpt === true ? { reuse_existing: true } : {}),
            }
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
      if (saved.kind === 'openai') await refreshOpenAiAccountStatus();
      setEditingProvider(null);
    } catch (providerError) {
      setError(
        providerError instanceof Error
          ? providerError.message
          : 'Rynna could not save the provider',
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
      if (kind === 'openai') await refreshOpenAiAccountStatus();
      setEditingProvider(null);
    } catch (providerError) {
      setError(
        providerError instanceof Error
          ? providerError.message
          : 'Rynna could not delete the provider',
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
      setError(requestError instanceof Error ? requestError.message : 'Rynna could not complete the request');
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
          <p className="eyebrow">AI software agent</p>
          <h1>Rynna</h1>
        </div>
        <div className="header-actions">
          {view === 'chat' && profiles.length > 0 ? (
            <label className="profile-picker" htmlFor="profile">
              <span>Profile</span>
              <Typeahead
                disabled={pending}
                id="profile"
                onChange={selectProfile}
                options={sortedProfiles(profiles).map((profile) => profile.name)}
                value={selectedProfile ?? ''}
              />
            </label>
          ) : null}
          {client.connectOpenAi ? (
            <Button
              className="account-button"
              onClick={() => {
                if (showOpenAi) setApiKey('');
                setShowOpenAi(!showOpenAi);
              }}
              type="button"
              variant="outline"
            >
              {openAiAccount?.connected
                ? openAiAccount.method === 'chatgpt'
                  ? `Connected with ChatGPT${openAiAccount.plan ? ` ${formatPlan(openAiAccount.plan)}` : ''}`
                  : 'Connected with API key'
                : 'Connect OpenAI'}
            </Button>
          ) : null}
          {canOpenSettings ? (
            <Button
              className="account-button"
              onClick={() => {
                setView(view === 'settings' ? 'chat' : 'settings');
                setEditingProvider(null);
                setError(null);
              }}
              type="button"
              variant="outline"
            >
              {view === 'settings' ? 'Back to chat' : 'Settings'}
            </Button>
          ) : null}
          <ThemeToggle />
          <span className="status"><span aria-hidden="true" /> Ready</span>
        </div>
      </header>

      {view === 'chat' && showOpenAi && client.connectOpenAi ? (
        <section className="account-panel" aria-label="Connect OpenAI">
          <Button
            disabled={connectingOpenAi}
            onClick={() => void connectOpenAi('chatgpt')}
            type="button"
            variant="secondary"
          >
            {connectingOpenAi ? 'Connecting…' : 'Use ChatGPT subscription'}
          </Button>
          <span>or</span>
          <label htmlFor="openai-api-key">OpenAI API key</label>
          <Input
            autoComplete="off"
            id="openai-api-key"
            onChange={(event) => setApiKey(event.target.value)}
            type="password"
            value={apiKey}
          />
          <Button
            disabled={connectingOpenAi || !apiKey.trim()}
            onClick={() => void connectOpenAi('api_key')}
            type="button"
          >
            Save API key
          </Button>
        </section>
      ) : null}

      {view === 'chat' && activeProfile ? (
        <aside className="profile-summary" aria-label="Active profile">
          {activeProfile.providers.map((provider, index) => (
            <span className="profile-provider-summary" key={`${provider.provider}-${provider.model}-${index}`}>
              <strong>{provider.model}</strong>
              <Badge>{provider.provider}</Badge>
            </span>
          ))}
          {activeProfile.active_skills.map((skill) => (
            <Badge key={`skill-${skill}`}>{skill} skill</Badge>
          ))}
          {activeProfile.mcp_servers.map((server) => (
            <Badge key={`mcp-${server}`}>{server} MCP</Badge>
          ))}
          {activeProfile.capabilities.map((capability) => (
            <Badge key={`capability-${capability}`}>{capability} capability</Badge>
          ))}
        </aside>
      ) : null}

      {view === 'settings' ? (
        <section className="settings-page" aria-label="Settings">
          <aside className="settings-sidebar">
            <p className="eyebrow">Settings</p>
            <nav aria-label="Settings">
              {canEditProfiles ? (
                <Button
                  aria-current={settingsSection === 'profiles' ? 'page' : undefined}
                  onClick={() => setSettingsSection('profiles')}
                  type="button"
                  variant="ghost"
                >
                  Profiles
                </Button>
              ) : null}
              {client.listProviders ? (
                <Button
                  aria-current={settingsSection === 'provider-credentials' ? 'page' : undefined}
                  onClick={() => setSettingsSection('provider-credentials')}
                  type="button"
                  variant="ghost"
                >
                  Provider credentials
                </Button>
              ) : null}
            </nav>
          </aside>
          <div className="settings-content">
            {settingsSection === 'profiles' && canEditProfiles ? (
              <>
                <div className="settings-heading">
                  <div>
                    <h2>Profiles</h2>
                    <p>Group ordered providers and models for each workflow.</p>
                  </div>
                  <div className="provider-actions">
                    <Button disabled={savingProfile} onClick={beginAddProfile} type="button">
                      Add profile
                    </Button>
                    <Button
                      disabled={savingProfile || addingProfile || profiles.length <= 1}
                      onClick={() => void removeProfile()}
                      type="button"
                      variant="ghost"
                    >
                      Delete profile
                    </Button>
                  </div>
                </div>
                {profiles.length === 0 && !addingProfile ? (
                  <p className="settings-empty">No profiles configured.</p>
                ) : (
                  <>
                    {addingProfile ? null : (
                      <label className="profile-picker" htmlFor="settings-profile">
                        <span>Profile</span>
                        <Typeahead
                          disabled={savingProfile}
                          id="settings-profile"
                          onChange={selectProfile}
                          options={sortedProfiles(profiles).map((profile) => profile.name)}
                          value={selectedProfile ?? ''}
                        />
                      </label>
                    )}
                    <form className="provider-form profile-form" onSubmit={(event) => void saveProfile(event)}>
                      <h3>{addingProfile ? 'Add profile' : 'Edit profile'}</h3>
                      <label htmlFor="profile-name">Name</label>
                      <Input
                        id="profile-name"
                        onChange={(event) => setProfileName(event.target.value)}
                        required
                        value={profileName}
                      />
                      <div className="profile-provider-heading">
                        <div>
                          <h4>Providers</h4>
                          <p>
                            Rynna tries these providers in order when a request fails. Restart Rynna before using saved profile changes.
                          </p>
                        </div>
                        <Button onClick={addProfileProvider} size="sm" type="button" variant="outline">
                          Add profile provider
                        </Button>
                      </div>
                      <div className="profile-provider-list">
                        {profileProviders.map((provider, index) => (
                          <fieldset className="profile-provider-row" key={index}>
                            <legend>Provider {index + 1}</legend>
                            <div>
                              <label htmlFor={`profile-provider-${index}`}>Provider</label>
                              <Typeahead
                                id={`profile-provider-${index}`}
                                onChange={(value) => updateProfileProvider(index, 'provider', value)}
                                options={profileProviderOptions(profileProviderIds)}
                                value={provider.provider}
                              />
                            </div>
                            <div>
                              <label htmlFor={`profile-model-${index}`}>Model</label>
                              <Input
                                id={`profile-model-${index}`}
                                onChange={(event) => updateProfileProvider(index, 'model', event.target.value)}
                                required
                                value={provider.model}
                              />
                            </div>
                            <Button
                              aria-label={`Remove provider ${index + 1}`}
                              disabled={profileProviders.length <= 1}
                              onClick={() => removeProfileProvider(index)}
                              size="sm"
                              type="button"
                              variant="ghost"
                            >
                              Remove
                            </Button>
                          </fieldset>
                        ))}
                      </div>
                      <div className="provider-actions">
                        <Button
                          disabled={
                            savingProfile ||
                            !profileName.trim() ||
                            profileProviders.length === 0 ||
                            profileProviders.some(
                              (provider) => !provider.provider.trim() || !provider.model.trim(),
                            )
                          }
                          type="submit"
                        >
                          Save profile
                        </Button>
                        {addingProfile ? (
                          <Button onClick={() => setAddingProfile(false)} type="button" variant="ghost">
                            Cancel
                          </Button>
                        ) : null}
                      </div>
                    </form>
                  </>
                )}
              </>
            ) : null}
            {settingsSection === 'provider-credentials' && client.listProviders ? (
              <>
                <div className="settings-heading">
                  <div>
                    <h2>Provider credentials</h2>
                    <p>Manage authentication shared by provider-backed profiles.</p>
                  </div>
                  <Button disabled={providerSettings.length >= 3} onClick={beginAddProvider} type="button">
                    Add provider
                  </Button>
                </div>
                <p>
                  Provider credentials are stored here; runtime profiles and models load from config.toml at startup.
                </p>
          {providerSettings.length === 0 ? (
            <p className="settings-empty">No providers configured.</p>
          ) : (
            <div className="provider-list">
              {[...providerSettings]
                .sort((left, right) =>
                  providerTitle(left.kind).localeCompare(providerTitle(right.kind)),
                )
                .map((provider) => (
                <article className="provider-card" key={provider.kind}>
                  <div>
                    <h3>{providerTitle(provider.kind)}</h3>
                    <p>
                      {provider.kind === 'ollama'
                        ? provider.api_base
                        : provider.kind === 'anthropic'
                          ? provider.authentication === 'subscription'
                            ? 'Claude subscription / usage bundle'
                            : 'API key via environment variable'
                          : provider.authentication === 'chatgpt'
                            ? 'ChatGPT subscription'
                            : 'API key'}
                    </p>
                  </div>
                  <div className="provider-actions">
                    <Button
                      aria-label={`Edit ${providerTitle(provider.kind)}`}
                      onClick={() => beginEditProvider(provider)}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      Edit
                    </Button>
                    <Button
                      aria-label={`Delete ${providerTitle(provider.kind)}`}
                      onClick={() => void removeProvider(provider.kind)}
                      size="sm"
                      type="button"
                      variant="ghost"
                    >
                      Delete
                    </Button>
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
              {providerSettings.some((provider) => provider.kind === editingProvider) ? (
                <select disabled id="provider-type" value={providerKind}>
                  <option value={providerKind}>{providerTitle(providerKind)}</option>
                </select>
              ) : (
                <div className="provider-typeahead">
                  <Input
                    aria-autocomplete="list"
                    aria-activedescendant={
                      providerTypeOpen && activeProviderKind
                        ? `provider-type-option-${activeProviderKind}`
                        : undefined
                    }
                    aria-controls="provider-type-options"
                    aria-expanded={providerTypeOpen}
                    autoComplete="off"
                    id="provider-type"
                    onBlur={() => {
                      setProviderTypeQuery(providerTitle(providerKind));
                      setProviderTypeOpen(false);
                      setProviderTypeDirty(false);
                      setActiveProviderKind(providerKind);
                    }}
                    onChange={(event) => {
                      const query = event.target.value;
                      setProviderTypeQuery(query);
                      setProviderTypeOpen(true);
                      setProviderTypeDirty(true);
                      setActiveProviderKind(matchingProviderKinds(query, false)[0] ?? null);
                    }}
                    onFocus={(event) => {
                      event.currentTarget.select();
                      setProviderTypeOpen(true);
                      const matches = matchingProviderKinds(providerTypeQuery);
                      setActiveProviderKind(matches.includes(providerKind) ? providerKind : (matches[0] ?? null));
                    }}
                    onKeyDown={(event) => {
                      if (event.key === 'Escape') {
                        setProviderTypeQuery(providerTitle(providerKind));
                        setProviderTypeOpen(false);
                        setProviderTypeDirty(false);
                        setActiveProviderKind(providerKind);
                        return;
                      }
                      const matches = matchingProviderKinds(providerTypeQuery);
                      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                        if (matches.length === 0) return;
                        event.preventDefault();
                        setProviderTypeOpen(true);
                        const currentIndex = activeProviderKind ? matches.indexOf(activeProviderKind) : -1;
                        const offset = event.key === 'ArrowDown' ? 1 : -1;
                        const nextIndex =
                          currentIndex === -1
                            ? event.key === 'ArrowDown' ? 0 : matches.length - 1
                            : (currentIndex + offset + matches.length) % matches.length;
                        setActiveProviderKind(matches[nextIndex] ?? null);
                        return;
                      }
                      if (event.key !== 'Enter') return;
                      const match =
                        (activeProviderKind && matches.includes(activeProviderKind)
                          ? activeProviderKind
                          : matches[0]) ?? null;
                      if (!match) return;
                      event.preventDefault();
                      selectProviderKind(match);
                    }}
                    role="combobox"
                    value={providerTypeQuery}
                  />
                  {providerTypeOpen ? (
                    <div className="provider-type-options" id="provider-type-options" role="listbox">
                      {matchingProviderKinds(providerTypeQuery).map((kind) => (
                        <button
                          aria-selected={kind === activeProviderKind}
                          id={`provider-type-option-${kind}`}
                          key={kind}
                          onClick={() => selectProviderKind(kind)}
                          onMouseDown={(event) => event.preventDefault()}
                          onMouseMove={() => setActiveProviderKind(kind)}
                          role="option"
                          tabIndex={-1}
                          type="button"
                        >
                          {providerTitle(kind)}
                        </button>
                      ))}
                    </div>
                  ) : null}
                </div>
              )}
              {providerKind === 'ollama' ? (
                <>
                  <label htmlFor="ollama-api-base">Ollama API base URL</label>
                  <Input
                    id="ollama-api-base"
                    onChange={(event) => setOllamaApiBase(event.target.value)}
                    required
                    type="url"
                    value={ollamaApiBase}
                  />
                </>
              ) : providerKind === 'openai' ? (
                <>
                  <label htmlFor="openai-authentication">OpenAI authentication</label>
                  <select
                    id="openai-authentication"
                    onChange={(event) => {
                      setOpenAiAuthentication(event.target.value as 'api_key' | 'chatgpt');
                      setReuseExistingChatgpt(null);
                    }}
                    value={openAiAuthentication}
                  >
                    <option value="chatgpt">ChatGPT subscription</option>
                    <option value="api_key">API key</option>
                  </select>
                  {openAiAuthentication === 'api_key' ? (
                    <>
                      <label htmlFor="provider-openai-api-key">OpenAI API key</label>
                      <Input
                        autoComplete="off"
                        id="provider-openai-api-key"
                        onChange={(event) => setProviderApiKey(event.target.value)}
                        required
                        type="password"
                        value={providerApiKey}
                      />
                    </>
                  ) : existingOpenAiAccount?.connected && existingOpenAiAccount.method === 'chatgpt' ? (
                    <div className="credential-choice">
                      <strong>Existing ChatGPT credentials found</strong>
                      <p>
                        {existingOpenAiAccount.plan
                          ? `ChatGPT ${formatPlan(existingOpenAiAccount.plan)}`
                          : 'A ChatGPT subscription'} is already connected. Use it or sign in with a
                        different account for Rynna.
                      </p>
                      <div className="provider-actions">
                        <Button
                          aria-pressed={reuseExistingChatgpt === true}
                          onClick={() => setReuseExistingChatgpt(true)}
                          type="button"
                          variant="outline"
                        >
                          Use existing credentials
                        </Button>
                        <Button
                          aria-pressed={reuseExistingChatgpt === false}
                          onClick={() => setReuseExistingChatgpt(false)}
                          type="button"
                          variant="outline"
                        >
                          Register new credentials
                        </Button>
                      </div>
                      {reuseExistingChatgpt === false ? (
                        <p>A browser window will open so you can sign in to ChatGPT.</p>
                      ) : null}
                    </div>
                  ) : (
                    <p>A browser window will open so you can sign in to ChatGPT.</p>
                  )}
                </>
              ) : (
                <>
                  <label htmlFor="anthropic-authentication">Anthropic authentication</label>
                  <select
                    id="anthropic-authentication"
                    onChange={(event) =>
                      setAnthropicAuthentication(event.target.value as 'api_key' | 'subscription')
                    }
                    value={anthropicAuthentication}
                  >
                    <option value="subscription">Claude subscription / usage bundle</option>
                    <option value="api_key">API key from ANTHROPIC_API_KEY</option>
                  </select>
                  <p>
                    {anthropicAuthentication === 'subscription'
                      ? 'A browser window will open for Claude login. Rynna tools are disabled for this mode.'
                      : 'Set ANTHROPIC_API_KEY in the Rynna process environment; the key is never saved in provider settings.'}
                  </p>
                </>
              )}
              <div className="provider-actions">
                <Button
                  disabled={
                    savingProvider ||
                    providerTypeQuery !== providerTitle(providerKind) ||
                    (providerKind === 'openai' &&
                      openAiAuthentication === 'chatgpt' &&
                      (discoveringExistingOpenAiAccount ||
                        (existingOpenAiAccount?.connected === true &&
                          existingOpenAiAccount.method === 'chatgpt' &&
                          reuseExistingChatgpt === null)))
                  }
                  type="submit"
                >
                  Save provider
                </Button>
                <Button
                  onClick={() => {
                    setEditingProvider(null);
                    setReuseExistingChatgpt(null);
                    setProviderApiKey('');
                  }}
                  type="button"
                  variant="ghost"
                >
                  Cancel
                </Button>
              </div>
            </form>
          ) : null}
              </>
            ) : null}
          {error ? <p className="request-error" role="alert">{error}</p> : null}
          </div>
        </section>
      ) : null}

      {view === 'chat' ? <section className="conversation" aria-label="Conversation">
        <div className="messages" role="log" aria-live="polite">
          {messages.length === 0 ? (
            <div className="empty-state">
              <p className="thread-mark" aria-hidden="true">A</p>
              <h2>What should we work through?</h2>
              <p>Ask Rynna to investigate, plan, or execute a development task.</p>
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
                  <p className="message-role">{message.role === 'assistant' ? 'Rynna' : 'You'}</p>
                  <p>{message.content}</p>
                </article>
              ),
            )
          )}
        </div>

        {error ? <p className="request-error" role="alert">{error}</p> : null}
        <form className="composer" onSubmit={submit}>
          <label htmlFor="prompt">Message Rynna</label>
          <div className="composer-row">
            <Textarea
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
            <Button disabled={pending || !input.trim()} type="submit">
              {pending ? 'Working…' : 'Send'}
            </Button>
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
  return kind === 'ollama' ? 'Ollama' : kind === 'openai' ? 'OpenAI' : 'Anthropic';
}
