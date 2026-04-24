import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/common/ProviderIconPicker', () => ({
  ProviderIconPicker: () => <div />,
  ProviderIconImg: () => <span />,
}));

import { ProviderHeadersEditor } from '../components/settings/ProviderHeadersEditor';
import {
  emptyProvider,
  jsonToProviders,
  providersToToml,
  type ProviderFormData,
} from '../components/settings/settingsTypes';

describe('provider custom headers settings', () => {
  it('serializes provider headers as key-value TOML entries and omits blank keys', () => {
    const provider: ProviderFormData = {
      ...emptyProvider(),
      id: 'gateway',
      provider_type: 'openai-compat',
      model: 'gateway-model',
      headers: {
        'X-LLM-Tenant': 'workspace-a',
        'HTTP-Referer': 'https://y-agent.local',
        '': 'ignored',
      },
    };

    const toml = providersToToml([provider]);

    expect(toml).toContain('[providers.headers]');
    expect(toml).toContain('"X-LLM-Tenant" = "workspace-a"');
    expect(toml).toContain('"HTTP-Referer" = "https://y-agent.local"');
    expect(toml).not.toContain('"" = "ignored"');
  });

  it('deserializes provider headers from parsed config JSON', () => {
    const providers = jsonToProviders({
      providers: {
        providers: [
          {
            id: 'gateway',
            provider_type: 'openai-compat',
            model: 'gateway-model',
            headers: {
              'X-LLM-Tenant': 'workspace-a',
              'X-Feature': 'strict-validation',
            },
          },
        ],
      },
    });

    expect(providers[0].headers).toEqual({
      'X-LLM-Tenant': 'workspace-a',
      'X-Feature': 'strict-validation',
    });
  });

  it('renders the provider header key-value editor', () => {
    const html = renderToStaticMarkup(
      <ProviderHeadersEditor
        headers={{ 'X-LLM-Tenant': 'workspace-a' }}
        onChange={() => {}}
      />,
    );

    expect(html).toContain('X-LLM-Tenant');
    expect(html).toContain('workspace-a');
    expect(html).toContain('Add Header');
  });
});
