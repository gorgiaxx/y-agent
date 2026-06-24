import { describe, expect, it } from 'vitest';

import {
  RETRY_DEFAULTS,
  retryToToml,
  jsonToRetry,
  stripRetrySection,
  buildProvidersToml,
  emptyProvider,
  type RetryFormData,
} from '../components/settings/settingsTypes';

describe('retry policy settings serialization', () => {
  it('serializes a [retry] table with all fields', () => {
    const form: RetryFormData = {
      enabled: true,
      max_retries: 5,
      initial_delay_ms: 2000,
      max_delay_ms: 60000,
      backoff: 'fixed',
    };
    const toml = retryToToml(form);
    expect(toml).toContain('[retry]');
    expect(toml).toContain('enabled = true');
    expect(toml).toContain('max_retries = 5');
    expect(toml).toContain('initial_delay_ms = 2000');
    expect(toml).toContain('max_delay_ms = 60000');
    expect(toml).toContain('backoff = "fixed"');
  });

  it('parses [retry] nested under providers and falls back to defaults', () => {
    const parsed = jsonToRetry({
      providers: { retry: { enabled: false, max_retries: 7, backoff: 'fixed' } },
    });
    expect(parsed.enabled).toBe(false);
    expect(parsed.max_retries).toBe(7);
    expect(parsed.backoff).toBe('fixed');
    // Missing fields fall back to defaults.
    expect(parsed.initial_delay_ms).toBe(RETRY_DEFAULTS.initial_delay_ms);
    expect(parsed.max_delay_ms).toBe(RETRY_DEFAULTS.max_delay_ms);
  });

  it('returns defaults when no [retry] table is present', () => {
    expect(jsonToRetry({ providers: { providers: [] } })).toEqual(RETRY_DEFAULTS);
    expect(jsonToRetry(null)).toEqual(RETRY_DEFAULTS);
  });

  it('strips an existing [retry] table from meta while preserving other content', () => {
    const meta = [
      'default_freeze_duration_secs = 30',
      '',
      '[retry]',
      'enabled = true',
      'max_retries = 9',
      '',
      '[proxy.global]',
      'url = "socks5://localhost:1080"',
      '',
    ].join('\n');
    const stripped = stripRetrySection(meta);
    expect(stripped).toContain('default_freeze_duration_secs = 30');
    expect(stripped).toContain('[proxy.global]');
    expect(stripped).toContain('url = "socks5://localhost:1080"');
    expect(stripped).not.toContain('[retry]');
    expect(stripped).not.toContain('max_retries = 9');
  });

  it('assembles providers.toml with meta, a single [retry] table, then providers', () => {
    const meta = 'default_freeze_duration_secs = 30\n';
    const form: RetryFormData = { ...RETRY_DEFAULTS, max_retries: 5 };
    const provider = { ...emptyProvider(), id: 'deepseek', provider_type: 'deepseek', model: 'deepseek-chat' };

    const toml = buildProvidersToml(meta, form, [provider]);

    // Order: top-level scalar, then [retry], then [[providers]].
    const freezeIdx = toml.indexOf('default_freeze_duration_secs');
    const retryIdx = toml.indexOf('[retry]');
    const providersIdx = toml.indexOf('[[providers]]');
    expect(freezeIdx).toBeGreaterThanOrEqual(0);
    expect(retryIdx).toBeGreaterThan(freezeIdx);
    expect(providersIdx).toBeGreaterThan(retryIdx);
    expect(toml).toContain('max_retries = 5');
    expect(toml).toContain('id = "deepseek"');

    // Exactly one [retry] table (no duplication with the preserved meta).
    expect(toml.match(/\[retry\]/g)?.length).toBe(1);
  });

  it('round-trips: stripping then rebuilding does not duplicate [retry]', () => {
    const original = [
      '[retry]',
      'enabled = true',
      'max_retries = 3',
      '',
      '[[providers]]',
      'id = "p"',
      '',
    ].join('\n');
    // Simulate load: capture meta-before-providers and strip retry from it.
    const firstTable = original.indexOf('[[providers]]');
    const meta = stripRetrySection(original.slice(0, firstTable));
    const rebuilt = buildProvidersToml(meta, RETRY_DEFAULTS, [
      { ...emptyProvider(), id: 'p' },
    ]);
    expect(rebuilt.match(/\[retry\]/g)?.length).toBe(1);
  });
});
