import { render, screen } from '@testing-library/react';
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
});
