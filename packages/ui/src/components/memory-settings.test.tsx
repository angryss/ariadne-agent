import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { MemorySettingsPanel } from './memory-settings';

const cloud = {
  kind: 'hindsight' as const,
  deployment: 'cloud' as const,
  api_base: 'https://api.hindsight.vectorize.io',
  bank_id: 'rynna',
  api_key_configured: true,
};

describe('Memory settings', () => {
  it('opens from Settings with None selected and reveals only the selected provider fields', async () => {
    const user = userEvent.setup();
    const save = vi.fn().mockResolvedValue({
      ...cloud,
      deployment: 'self_hosted',
      api_base: 'http://localhost:8888',
      api_key_configured: false,
    });
    render(
      <App
        client={{
          respond: vi.fn(),
          getMemorySettings: vi.fn().mockResolvedValue({ kind: 'none' }),
          saveMemorySettings: save,
        }}
      />,
    );
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const provider = await screen.findByLabelText('Memory provider');
    expect(provider).toHaveValue('none');
    expect(screen.queryByLabelText('API URL')).not.toBeInTheDocument();
    await user.selectOptions(provider, 'hindsight');
    expect(screen.getByLabelText('Hosting')).toHaveValue('cloud');
    expect(screen.getByLabelText('API URL')).toHaveValue(cloud.api_base);
    expect(screen.getByLabelText('API URL')).toHaveAttribute('readonly');
    expect(screen.getByLabelText('API key')).toBeRequired();
    await user.selectOptions(screen.getByLabelText('Hosting'), 'self_hosted');
    expect(screen.getByLabelText('API URL')).toHaveValue(
      'http://localhost:8888',
    );
    expect(screen.getByLabelText('API key (optional)')).not.toBeRequired();
    await user.type(screen.getByLabelText('Memory bank ID'), 'rynna');
    await user.click(
      screen.getByRole('button', { name: 'Save memory settings' }),
    );
    expect(save).toHaveBeenCalledWith({
      kind: 'hindsight',
      deployment: 'self_hosted',
      api_base: 'http://localhost:8888',
      bank_id: 'rynna',
    });
    expect(await screen.findByRole('status')).toHaveTextContent(
      'Changes apply to your next request',
    );
    await user.selectOptions(provider, 'none');
    expect(screen.queryByLabelText('Memory bank ID')).not.toBeInTheDocument();
    await user.click(
      screen.getByRole('button', { name: 'Save memory settings' }),
    );
    expect(save).toHaveBeenLastCalledWith({ kind: 'none' });
  });

  it('keeps existing keys on blank saves and clears entered keys after saving', async () => {
    const user = userEvent.setup();
    const save = vi.fn().mockResolvedValue(cloud);
    render(
      <MemorySettingsPanel
        client={{
          respond: vi.fn(),
          getMemorySettings: vi.fn().mockResolvedValue(cloud),
          saveMemorySettings: save,
        }}
      />,
    );
    await screen.findByText('A key is saved. Leave this blank to keep it.');
    expect(screen.getByLabelText('API key')).toHaveValue('');
    await user.click(
      screen.getByRole('button', { name: 'Save memory settings' }),
    );
    expect(save).toHaveBeenLastCalledWith({
      kind: 'hindsight',
      deployment: 'cloud',
      api_base: cloud.api_base,
      bank_id: 'rynna',
    });
    await user.type(screen.getByLabelText('API key'), 'replacement-test-key');
    await user.click(
      screen.getByRole('button', { name: 'Save memory settings' }),
    );
    expect(save).toHaveBeenLastCalledWith(
      expect.objectContaining({ api_key: 'replacement-test-key' }),
    );
    expect(screen.getByLabelText('API key')).toHaveValue('');
  });

  it('reports failed saves without claiming success', async () => {
    const user = userEvent.setup();
    render(
      <MemorySettingsPanel
        client={{
          respond: vi.fn(),
          getMemorySettings: vi.fn().mockResolvedValue(cloud),
          saveMemorySettings: vi
            .fn()
            .mockRejectedValue(new Error('Could not save memory settings')),
        }}
      />,
    );
    await screen.findByText('A key is saved. Leave this blank to keep it.');
    await user.click(
      screen.getByRole('button', { name: 'Save memory settings' }),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Could not save memory settings',
    );
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });
});
