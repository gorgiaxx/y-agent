import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../lib', () => ({ transport: { invoke } }));

import {
  describeCatalogModel,
  formatTokens,
  searchModelCatalog,
  updateModelCatalog,
  type CatalogModel,
} from '../utils/modelCatalog';
import { COMMAND_MAP } from '../lib/commandMap';

const OPUS: CatalogModel = {
  provider_id: 'anthropic',
  provider_name: 'Anthropic',
  provider_api: 'https://api.anthropic.com/v1',
  provider_env: ['ANTHROPIC_API_KEY'],
  id: 'claude-opus-4-6',
  name: 'Claude Opus 4.6',
  tool_call: true,
  reasoning: true,
  capabilities: ['text', 'vision'],
  context_window: 200_000,
  max_output_tokens: 64_000,
  cost_per_1k_input: 0.005,
  cost_per_1k_output: 0.025,
  release_date: '2026-02-04',
  knowledge: '2025-05-31',
  canonical: true,
};

describe('models.dev catalog client', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({ fetched_at: null, total: 0, matches: [] });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('sends the update command with no explicit source url by default', async () => {
    invoke.mockResolvedValue({
      path: '/home/u/.config/y-agent/models.dev.json',
      source_url: 'https://models.dev/api.json',
      provider_count: 180,
      model_count: 6106,
      fetched_at: '2026-08-05T00:00:00Z',
      bytes: 3_536_613,
    });

    const summary = await updateModelCatalog();

    expect(invoke).toHaveBeenCalledWith('model_catalog_update', { sourceUrl: null });
    expect(summary.model_count).toBe(6106);
  });

  it('delegates fuzzy matching to the backend with the provider hint', async () => {
    await searchModelCatalog('[Kiro] claude-opus-4-6', 'anthropic', 25);

    expect(invoke).toHaveBeenCalledWith('model_catalog_search', {
      query: '[Kiro] claude-opus-4-6',
      providerType: 'anthropic',
      limit: 25,
    });
  });

  it('maps both catalog commands onto y-web REST endpoints with snake_case bodies', () => {
    const update = COMMAND_MAP.model_catalog_update;
    const search = COMMAND_MAP.model_catalog_search;

    expect(update.path).toBe('/api/v1/models/catalog/update');
    expect(update.body?.({ sourceUrl: 'https://example.test/api.json' }))
      .toEqual({ source_url: 'https://example.test/api.json' });

    expect(search.path).toBe('/api/v1/models/catalog/search');
    expect(search.body?.({ query: 'gpt-4o', providerType: 'openai', limit: 10 }))
      .toEqual({ query: 'gpt-4o', provider_type: 'openai', limit: 10 });
  });

  it('summarizes a catalog entry for the picker list', () => {
    expect(describeCatalogModel(OPUS))
      .toBe('Anthropic \u00b7 200K ctx \u00b7 tools \u00b7 reasoning \u00b7 vision');
  });

  it('formats token counts compactly', () => {
    expect(formatTokens(999)).toBe('999');
    expect(formatTokens(128_000)).toBe('128K');
    expect(formatTokens(200_000)).toBe('200K');
    expect(formatTokens(1_048_576)).toBe('1M');
    expect(formatTokens(1_500_000)).toBe('1.5M');
    expect(formatTokens(2_000_000)).toBe('2M');
  });
});
