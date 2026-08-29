import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from './App';
import type { AgentClient } from './contracts';

describe('App', () => {
  it('sends a prompt through the injected client and renders the reply', async () => {
    const client: AgentClient = {
      respond: vi.fn().mockResolvedValue({
        message: { role: 'assistant', content: 'Follow the thread.' },
      }),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Help me plan this');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(client.respond).toHaveBeenCalledWith(
      {
        prompt: 'Help me plan this',
        history: [],
      },
      expect.any(Function),
    );
    expect(await screen.findByText('Follow the thread.')).toBeInTheDocument();
    expect(screen.getByText('Help me plan this')).toBeInTheDocument();
  });

  it('submits the prompt when Enter is pressed in the composer', async () => {
    const respond = vi.fn().mockResolvedValue({
      message: { role: 'assistant' as const, content: 'Submitted.' },
    });
    const user = userEvent.setup();
    render(<App client={{ respond }} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Send with Enter{Enter}');

    expect(respond).toHaveBeenCalledWith(
      {
        prompt: 'Send with Enter',
        history: [],
      },
      expect.any(Function),
    );
    expect(await screen.findByText('Submitted.')).toBeInTheDocument();
  });

  it('collapses streamed thinking when user-facing content begins and lets the user expand it', async () => {
    const client: AgentClient = {
      respond: vi.fn(async (_request, onDelta) => {
        onDelta?.({ kind: 'thinking', content: 'Inspect the request' });
        onDelta?.({ kind: 'thinking', content: '\nCompare the fields' });
        onDelta?.({ kind: 'content', content: 'Here is the result.' });
        return { message: { role: 'assistant' as const, content: 'Here is the result.' } };
      }),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Investigate this');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByText('Here is the result.')).toBeInTheDocument();
    const disclosure = screen.getByText('Thinking').closest('details');
    expect(disclosure).not.toHaveAttribute('open');

    await user.click(screen.getByText('Thinking'));

    expect(disclosure).toHaveAttribute('open');
    expect(screen.getByText(/Inspect the request/)).toHaveTextContent(
      'Inspect the request Compare the fields',
    );
  });

  it('renders the final response and collapses thinking when no content delta arrives', async () => {
    const client: AgentClient = {
      respond: vi.fn(async (_request, onDelta) => {
        onDelta?.({ kind: 'thinking', content: 'Call sw_vers' });
        onDelta?.({ kind: 'content', content: '' });
        return {
          message: {
            role: 'assistant' as const,
            content: 'The computer is running macOS 26.6.1.',
          },
        };
      }),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Which operating system?');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByText('The computer is running macOS 26.6.1.')).toBeInTheDocument();
    expect(screen.getByText('Thinking').closest('details')).not.toHaveAttribute('open');
  });

  it('replaces a streamed draft with the authoritative final response', async () => {
    const client: AgentClient = {
      respond: vi.fn(async (_request, onDelta) => {
        onDelta?.({ kind: 'content', content: 'Draft answer' });
        return {
          message: { role: 'assistant' as const, content: 'Verified final answer' },
        };
      }),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Answer this');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByText('Verified final answer')).toBeInTheDocument();
    expect(screen.queryByText('Draft answer')).not.toBeInTheDocument();
  });

  it('shows a recoverable error when the client request fails', async () => {
    const client: AgentClient = {
      respond: vi.fn().mockRejectedValue(new Error('The local server is unavailable')),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Try this');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('The local server is unavailable');
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled();
  });

  it('retries a failed prompt without duplicating it in conversation history', async () => {
    const respond = vi
      .fn()
      .mockRejectedValueOnce(new Error('Try again'))
      .mockResolvedValueOnce({
        message: { role: 'assistant' as const, content: 'Recovered.' },
      });
    const user = userEvent.setup();
    render(<App client={{ respond }} />);

    await user.type(screen.getByLabelText('Message Ariadne'), 'Retry this');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await screen.findByRole('alert');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(respond).toHaveBeenNthCalledWith(
      2,
      {
        prompt: 'Retry this',
        history: [],
      },
      expect.any(Function),
    );
    expect(await screen.findByText('Recovered.')).toBeInTheDocument();
    expect(screen.getAllByText('Retry this')).toHaveLength(1);
  });

  it('lists profiles and sends new conversations through the selected profile', async () => {
    const respond = vi.fn().mockResolvedValue({
      message: { role: 'assistant' as const, content: 'Work reply.' },
    });
    const client: AgentClient = {
      listProfiles: vi.fn().mockResolvedValue({
        default_profile: 'local',
        profiles: [
          {
            name: 'local',
            provider: 'ollama',
            model: 'qwen3:8b',
            active_skills: [],
            mcp_servers: [],
            capabilities: ['workspace'],
          },
          {
            name: 'work',
            provider: 'openai',
            model: 'gpt-5',
            active_skills: ['github'],
            mcp_servers: ['github'],
            capabilities: [],
          },
        ],
      }),
      respond,
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    const profile = await screen.findByLabelText('Profile');
    expect(screen.getByText('workspace capability')).toBeInTheDocument();
    await user.type(screen.getByLabelText('Message Ariadne'), 'Use local');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await screen.findByText('Work reply.');

    await user.selectOptions(profile, 'work');
    await user.type(screen.getByLabelText('Message Ariadne'), 'Use work');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(respond).toHaveBeenNthCalledWith(
      2,
      {
        profile: 'work',
        prompt: 'Use work',
        history: [],
      },
      expect.any(Function),
    );
    expect(screen.getByText('gpt-5')).toBeInTheDocument();
    expect(screen.getByText('github skill')).toBeInTheDocument();
    expect(screen.getByText('github MCP')).toBeInTheDocument();
  });

  it('connects a ChatGPT subscription or API key without exposing the key', async () => {
    const connectOpenAi = vi.fn().mockResolvedValue({
      connected: true,
      method: 'api_key' as const,
    });
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockResolvedValue({ connected: false, method: null }),
      connectOpenAi,
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Connect OpenAI' }));
    expect(screen.getByRole('button', { name: 'Use ChatGPT subscription' })).toBeInTheDocument();

    await user.type(screen.getByLabelText('OpenAI API key'), 'sk-secret-value');
    expect(screen.getByLabelText('OpenAI API key')).toHaveAttribute('type', 'password');
    await user.click(screen.getByRole('button', { name: 'Save API key' }));

    expect(connectOpenAi).toHaveBeenCalledWith({ method: 'api_key', api_key: 'sk-secret-value' });
    expect(screen.queryByDisplayValue('sk-secret-value')).not.toBeInTheDocument();
    expect(await screen.findByText('Connected with API key')).toBeInTheDocument();
  });

  it('starts ChatGPT browser sign-in and reports the connected plan', async () => {
    const connectOpenAi = vi.fn().mockResolvedValue({
      connected: true,
      method: 'chatgpt' as const,
      plan: 'plus',
    });
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockResolvedValue({ connected: false, method: null }),
      connectOpenAi,
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Connect OpenAI' }));
    await user.click(screen.getByRole('button', { name: 'Use ChatGPT subscription' }));

    expect(connectOpenAi).toHaveBeenCalledWith({ method: 'chatgpt' });
    expect(await screen.findByText('Connected with ChatGPT Plus')).toBeInTheDocument();
  });

  it('does not let stale initial account status overwrite a completed connection', async () => {
    let resolveInitialAccount!: (account: { connected: false; method: null }) => void;
    const initialAccount = new Promise<{ connected: false; method: null }>((resolve) => {
      resolveInitialAccount = resolve;
    });
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockReturnValue(initialAccount),
      connectOpenAi: vi.fn().mockResolvedValue({
        connected: true,
        method: 'chatgpt' as const,
        plan: 'plus',
      }),
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.click(screen.getByRole('button', { name: 'Connect OpenAI' }));
    await user.click(screen.getByRole('button', { name: 'Use ChatGPT subscription' }));
    expect(await screen.findByText('Connected with ChatGPT Plus')).toBeInTheDocument();

    resolveInitialAccount({ connected: false, method: null });

    expect(await screen.findByText('Connected with ChatGPT Plus')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Connect OpenAI' })).not.toBeInTheDocument();
  });

  it('clears a rejected API key from the UI', async () => {
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockResolvedValue({ connected: false, method: null }),
      connectOpenAi: vi.fn().mockRejectedValue(new Error('OpenAI rejected the key')),
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Connect OpenAI' }));
    const input = screen.getByLabelText('OpenAI API key');
    await user.type(input, '«redacted:sk-…»');
    await user.click(screen.getByRole('button', { name: 'Save API key' }));

    expect(await screen.findByText('OpenAI rejected the key')).toBeInTheDocument();
    expect(input).toHaveValue('');
  });

  it('clears an API key when the account panel is dismissed', async () => {
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockResolvedValue({ connected: false, method: null }),
      connectOpenAi: vi.fn(),
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    const accountButton = await screen.findByRole('button', { name: 'Connect OpenAI' });
    await user.click(accountButton);
    await user.type(screen.getByLabelText('OpenAI API key'), '«redacted:sk-…»');
    await user.click(accountButton);
    await user.click(accountButton);

    expect(screen.getByLabelText('OpenAI API key')).toHaveValue('');
  });

  it('clears a typed API key when ChatGPT sign-in succeeds', async () => {
    const client: AgentClient = {
      getOpenAiAccount: vi.fn().mockResolvedValue({ connected: false, method: null }),
      connectOpenAi: vi.fn().mockResolvedValue({
        connected: true,
        method: 'chatgpt' as const,
        plan: 'plus',
      }),
      respond: vi.fn(),
    };
    const user = userEvent.setup();
    render(<App client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Connect OpenAI' }));
    await user.type(screen.getByLabelText('OpenAI API key'), '«redacted:sk-…»');
    await user.click(screen.getByRole('button', { name: 'Use ChatGPT subscription' }));
    await user.click(await screen.findByRole('button', { name: 'Connected with ChatGPT Plus' }));

    expect(screen.getByLabelText('OpenAI API key')).toHaveValue('');
  });

  it('opens provider settings blank and adds updates and deletes Ollama', async () => {
    const listProviders = vi.fn().mockResolvedValue([]);
    const createProvider = vi.fn().mockResolvedValue({
      kind: 'ollama' as const,
      api_base: 'http://localhost:11434/v1',
    });
    const updateProvider = vi.fn().mockResolvedValue({
      kind: 'ollama' as const,
      api_base: 'http://localhost:22434/v1',
    });
    const deleteProvider = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(
      <App
        client={{
          respond: vi.fn(),
          listProviders,
          createProvider,
          updateProvider,
          deleteProvider,
        }}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('heading', { name: 'Providers' })).toBeInTheDocument();
    expect(screen.getByText('No providers configured.')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Add provider' }));
    await user.selectOptions(screen.getByLabelText('Provider type'), 'ollama');
    await user.clear(screen.getByLabelText('Ollama API base URL'));
    await user.type(screen.getByLabelText('Ollama API base URL'), 'http://localhost:11434/v1');
    await user.click(screen.getByRole('button', { name: 'Save provider' }));
    expect(createProvider).toHaveBeenCalledWith({
      kind: 'ollama',
      api_base: 'http://localhost:11434/v1',
    });

    await user.click(screen.getByRole('button', { name: 'Edit Ollama' }));
    await user.clear(screen.getByLabelText('Ollama API base URL'));
    await user.type(screen.getByLabelText('Ollama API base URL'), 'http://localhost:22434/v1');
    await user.click(screen.getByRole('button', { name: 'Save provider' }));
    expect(updateProvider).toHaveBeenCalledWith({
      kind: 'ollama',
      api_base: 'http://localhost:22434/v1',
    });

    await user.click(screen.getByRole('button', { name: 'Delete Ollama' }));
    expect(deleteProvider).toHaveBeenCalledWith('ollama');
    expect(await screen.findByText('No providers configured.')).toBeInTheDocument();
  });

  it('adds OpenAI with either an API key or ChatGPT subscription', async () => {
    const createProvider = vi.fn().mockImplementation(async (provider) => provider);
    const user = userEvent.setup();
    render(
      <App
        client={{
          respond: vi.fn(),
          listProviders: vi.fn().mockResolvedValue([]),
          createProvider,
          updateProvider: vi.fn(),
          deleteProvider: vi.fn(),
        }}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('button', { name: 'Add provider' }));
    await user.selectOptions(screen.getByLabelText('Provider type'), 'openai');
    await user.selectOptions(screen.getByLabelText('OpenAI authentication'), 'api_key');
    await user.type(screen.getByLabelText('OpenAI API key'), 'sk-secret');
    await user.click(screen.getByRole('button', { name: 'Save provider' }));

    expect(createProvider).toHaveBeenCalledWith({
      kind: 'openai',
      authentication: 'api_key',
      api_key: 'sk-secret',
    });
    expect(screen.queryByDisplayValue('sk-secret')).not.toBeInTheDocument();
  });

  it('does not let a late initial provider list overwrite a completed mutation', async () => {
    let resolveProviders: (providers: []) => void = () => {};
    const listProviders = vi.fn(
      () =>
        new Promise<[]>((resolve) => {
          resolveProviders = resolve;
        }),
    );
    const createProvider = vi.fn().mockResolvedValue({
      kind: 'ollama' as const,
      api_base: 'http://localhost:11434/v1',
    });
    const user = userEvent.setup();
    render(
      <App
        client={{
          respond: vi.fn(),
          listProviders,
          createProvider,
          updateProvider: vi.fn(),
          deleteProvider: vi.fn(),
        }}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Add provider' }));
    await user.click(screen.getByRole('button', { name: 'Save provider' }));
    expect(await screen.findByRole('button', { name: 'Edit Ollama' })).toBeInTheDocument();

    await act(async () => resolveProviders([]));

    expect(screen.getByRole('button', { name: 'Edit Ollama' })).toBeInTheDocument();
  });
});
