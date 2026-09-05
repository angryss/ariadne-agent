import type { ModelSelection, Profile } from '../contracts';

export function ModelSelector({ profile, selection, disabled, onChange }: {
  profile: Profile;
  selection?: ModelSelection;
  disabled: boolean;
  onChange: (selection?: ModelSelection) => void;
}) {
  const pairs = profile.providers.filter(pair => pair.enabled !== false);
  const providers = [...new Set(pairs.map(pair => pair.provider))];
  return (
    <fieldset className="chat-model-selector" disabled={disabled}>
      <legend>Model for this conversation</legend>
      <label>Provider
        <select value={selection?.provider ?? ''} onChange={event => {
          const pair = pairs.find(pair => pair.provider === event.target.value);
          onChange(pair ? { provider: pair.provider, model: pair.model, thinking: 'default' } : undefined);
        }}>
          <option value="">Profile default</option>
          {providers.map(provider => <option key={provider}>{provider}</option>)}
        </select>
      </label>
      <label>Model
        <select disabled={!selection} value={selection?.model ?? ''} onChange={event => {
          if (selection) onChange({ ...selection, model: event.target.value, thinking: 'default' });
        }}>
          {!selection ? <option value="">Profile default</option> : pairs.filter(pair => pair.provider === selection.provider)
            .map(pair => <option key={pair.model}>{pair.model}</option>)}
        </select>
      </label>
      <label>Thinking level
        <select disabled={!selection} value={selection?.thinking ?? 'default'} onChange={event => {
          if (selection) onChange({ ...selection, thinking: event.target.value as ModelSelection['thinking'] });
        }}>
          <option value="default">Default</option>
          <option value="low">Low</option>
          <option value="medium">Medium</option>
          <option value="high">High</option>
        </select>
      </label>
      <p>Applies to the next message. Thinking support depends on the model.</p>
    </fieldset>
  );
}
