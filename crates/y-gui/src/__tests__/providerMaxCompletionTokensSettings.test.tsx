import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

vi.mock('../components/common/ProviderIconPicker', () => ({
  ProviderIconPicker: () => <div />,
  ProviderIconImg: () => <span />,
}));

import {
  emptyProvider,
  jsonToProviders,
  providersToToml,
  type ProviderFormData,
} from '../components/settings/settingsTypes';

describe('provider use_max_completion_tokens settings', () => {
  it('supports a provider-level maximum output token default', () => {
    const provider: ProviderFormData = {
      ...emptyProvider(),
      id: 'long-context',
      model: 'large-model',
      max_output_tokens: 32_000,
    };

    expect(providersToToml([provider])).toContain('max_output_tokens = 32000');
    expect(jsonToProviders({ providers: [provider] })[0].max_output_tokens).toBe(32_000);
    const source = readFileSync(
      new URL('../components/settings/ProvidersTab.tsx', import.meta.url),
      'utf8',
    );
    expect(source).toContain('title="Max Output Tokens"');
  });

  it('defaults to null on new providers and omits the field in TOML', () => {
    const provider = emptyProvider();
    expect(provider.use_max_completion_tokens).toBeNull();

    const toml = providersToToml([provider]);
    expect(toml).not.toContain('use_max_completion_tokens');
  });

  it('serializes use_max_completion_tokens = true when opted in', () => {
    const provider: ProviderFormData = {
      ...emptyProvider(),
      id: 'o3-reasoning',
      provider_type: 'openai',
      model: 'o3',
      use_max_completion_tokens: true,
    };

    const toml = providersToToml([provider]);
    expect(toml).toContain('use_max_completion_tokens = true');
  });

  it('round-trips through providersToToml + jsonToProviders', () => {
    const providers = jsonToProviders({
      providers: {
        providers: [
          {
            id: 'modern',
            provider_type: 'openai',
            model: 'o3',
            use_max_completion_tokens: true,
          },
          {
            id: 'legacy',
            provider_type: 'openai-compat',
            model: 'gpt-4o',
          },
        ],
      },
    });

    expect(providers[0].use_max_completion_tokens).toBe(true);
    // Absent in source JSON => null in form model (means follow Rust default).
    expect(providers[1].use_max_completion_tokens).toBeNull();
  });

  it('preserves an explicit false from existing TOML so the user can override', () => {
    const providers = jsonToProviders({
      providers: {
        providers: [
          {
            id: 'pinned-legacy',
            provider_type: 'openai',
            model: 'gpt-4o',
            use_max_completion_tokens: false,
          },
        ],
      },
    });

    expect(providers[0].use_max_completion_tokens).toBe(false);
    // Serializing an explicit false preserves the override in TOML.
    expect(providersToToml(providers)).toContain('use_max_completion_tokens = false');
  });
});
