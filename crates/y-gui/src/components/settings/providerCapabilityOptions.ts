// ---------------------------------------------------------------------------
// Provider capability options -- mirrors the Rust `ProviderCapability` enum
// (crates/y-core/src/provider.rs). Capabilities are a closed set, so the UI
// offers a selection list instead of free-text tags.
// ---------------------------------------------------------------------------

export interface ProviderCapabilityOption {
  /** Serde value written to providers.toml. */
  value: string;
  label: string;
  description: string;
}

export const PROVIDER_CAPABILITY_OPTIONS: readonly ProviderCapabilityOption[] = [
  {
    value: 'text',
    label: 'Text',
    description: 'Text chat completions',
  },
  {
    value: 'vision',
    label: 'Vision',
    description: 'Accepts image input',
  },
  {
    value: 'image_generation',
    label: 'Image Generation',
    description: 'Produces images',
  },
] as const;

/** Capability values the backend understands. */
export const PROVIDER_CAPABILITY_VALUES: readonly string[] =
  PROVIDER_CAPABILITY_OPTIONS.map((option) => option.value);

/**
 * Drop values the backend does not understand and de-duplicate, preserving the
 * canonical option order so hand-edited TOML normalizes on load.
 */
export function normalizeCapabilities(values: readonly string[]): string[] {
  return PROVIDER_CAPABILITY_VALUES.filter((value) => values.includes(value));
}
