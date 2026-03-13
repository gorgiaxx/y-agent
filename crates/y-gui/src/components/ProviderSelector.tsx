import type { ProviderInfo } from '../types';
import './ProviderSelector.css';

interface ProviderSelectorProps {
  providers: ProviderInfo[];
  selectedProviderId: string;
  onSelect: (id: string) => void;
  disabled: boolean;
}

export function ProviderSelector({
  providers,
  selectedProviderId,
  onSelect,
  disabled,
}: ProviderSelectorProps) {
  return (
    <select
      className="provider-selector"
      value={selectedProviderId}
      onChange={(e) => onSelect(e.target.value)}
      disabled={disabled}
      title="Select provider"
      id="provider-selector"
    >
      <option value="auto">Auto</option>
      {providers.map((p) => (
        <option key={p.id} value={p.id}>
          {p.id} ({p.model})
        </option>
      ))}
    </select>
  );
}
