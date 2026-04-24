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
      http_protocol: 'http2',
    };

    const toml = providersToToml([provider]);

    expect(toml).toContain('[providers.headers]');
    expect(toml).toContain('"X-LLM-Tenant" = "workspace-a"');
    expect(toml).toContain('"HTTP-Referer" = "https://y-agent.local"');
    expect(toml).toContain('http_protocol = "http2"');
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
            http_protocol: 'http2',
          },
        ],
      },
    });

    expect(providers[0].headers).toEqual({
      'X-LLM-Tenant': 'workspace-a',
      'X-Feature': 'strict-validation',
    });
    expect(providers[0].http_protocol).toBe('http2');
  });

  it('defaults provider HTTP protocol to HTTP/1.1', () => {
    const providers = jsonToProviders({
      providers: {
        providers: [{ id: 'default', provider_type: 'openai', model: 'gpt-4o' }],
      },
    });

    expect(providers[0].http_protocol).toBe('http1');
    expect(providersToToml(providers)).not.toContain('http_protocol');
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
