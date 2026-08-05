// ---------------------------------------------------------------------------
// CapabilitySelect -- toggle list for the closed set of provider capabilities.
//
// Replaces free-text tag entry: users pick from the values the backend
// actually understands, so a typo can no longer silently disable a capability.
// ---------------------------------------------------------------------------

import { Check } from 'lucide-react';

import {
  PROVIDER_CAPABILITY_OPTIONS,
  normalizeCapabilities,
} from './providerCapabilityOptions';

export function CapabilitySelect({
  selected,
  onChange,
}: {
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  const toggle = (value: string) => {
    const next = selected.includes(value)
      ? selected.filter((entry) => entry !== value)
      : [...selected, value];
    onChange(normalizeCapabilities(next));
  };

  return (
    <div className="flex flex-wrap gap-1.5" role="group" aria-label="Provider capabilities">
      {PROVIDER_CAPABILITY_OPTIONS.map((option) => {
        const active = selected.includes(option.value);
        return (
          <button
            key={option.value}
            type="button"
            role="checkbox"
            aria-checked={active}
            title={option.description}
            onClick={() => toggle(option.value)}
            className={[
              'inline-flex items-center gap-1',
              'px-2 py-1 rounded-[var(--radius-sm)]',
              'border border-solid text-12px font-sans',
              'transition-colors duration-150 cursor-pointer',
              active
                ? 'border-[var(--accent)] text-[var(--text-primary)] bg-[var(--accent-subtle)]'
                : 'border-[var(--border)] text-[var(--text-muted)] bg-transparent',
            ].join(' ')}
          >
            {active && <Check size={11} aria-hidden="true" />}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
