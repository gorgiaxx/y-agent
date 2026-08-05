import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { CapabilitySelect } from '../components/settings/CapabilitySelect';
import {
  PROVIDER_CAPABILITY_OPTIONS,
  PROVIDER_CAPABILITY_VALUES,
  normalizeCapabilities,
} from '../components/settings/providerCapabilityOptions';
import { jsonToProviders, providersToToml } from '../components/settings/settingsTypes';

describe('provider capability selection', () => {
  it('offers exactly the capabilities the Rust ProviderCapability enum accepts', () => {
    expect(PROVIDER_CAPABILITY_VALUES).toEqual(['text', 'vision', 'image_generation']);
  });

  it('drops values the backend cannot deserialize and de-duplicates', () => {
    expect(normalizeCapabilities(['vision', 'tool_call', 'vision'])).toEqual(['vision']);
    expect(normalizeCapabilities(['image_generation', 'text']))
      .toEqual(['text', 'image_generation']);
    expect(normalizeCapabilities([])).toEqual([]);
  });

  it('normalizes hand-edited TOML capabilities when loading the form', () => {
    const providers = jsonToProviders({
      providers: {
        providers: [
          {
            id: 'typo',
            provider_type: 'openai',
            model: 'gpt-4o',
            capabilities: ['visoin', 'text'],
          },
        ],
      },
    });

    expect(providers[0].capabilities).toEqual(['text']);
    expect(providersToToml(providers)).toContain('capabilities = ["text"]');
  });

  it('renders one toggle per capability and marks the selected ones', () => {
    const markup = renderToStaticMarkup(
      <CapabilitySelect selected={['vision']} onChange={() => {}} />,
    );

    for (const option of PROVIDER_CAPABILITY_OPTIONS) {
      expect(markup).toContain(option.label);
    }
    expect(markup.match(/role="checkbox"/g)).toHaveLength(3);
    expect(markup.match(/aria-checked="true"/g)).toHaveLength(1);
    expect(markup).toMatch(/aria-checked="true"[^>]*title="Accepts image input"/);
  });

  it('replaced the free-text chip input in the provider form', () => {
    const source = readFileSync(
      new URL('../components/settings/ProvidersTab.tsx', import.meta.url),
      'utf8',
    );
    const capabilitiesBlock = source
      .slice(source.indexOf('title="Capabilities"'))
      .slice(0, 400);

    expect(capabilitiesBlock).toContain('<CapabilitySelect');
    expect(capabilitiesBlock).not.toContain('<TagChipInput');
  });
});
