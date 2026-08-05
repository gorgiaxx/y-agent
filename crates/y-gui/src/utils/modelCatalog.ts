// ---------------------------------------------------------------------------
// modelCatalog -- client for the cached models.dev catalog (backend-owned).
//
// The backend downloads models.dev/api.json into the XDG config directory and
// performs the fuzzy id matching, so desktop and web behave identically and
// the matching rules live in exactly one place.
// ---------------------------------------------------------------------------

import { transport } from '../lib';

export interface CatalogModel {
  provider_id: string;
  provider_name: string;
  provider_api: string | null;
  provider_env: string[];
  id: string;
  name: string;
  tool_call: boolean;
  reasoning: boolean;
  capabilities: string[];
  context_window: number | null;
  max_output_tokens: number | null;
  cost_per_1k_input: number | null;
  cost_per_1k_output: number | null;
  release_date: string | null;
  knowledge: string | null;
  /** True when this provider is the model's first-party home. */
  canonical: boolean;
}

export interface CatalogMatch {
  score: number;
  /** True when the score clears the backend's confident-resolution threshold. */
  resolved: boolean;
  model: CatalogModel;
}

export interface CatalogSearchResult {
  /** RFC 3339 timestamp of the cached catalog, or null when never downloaded. */
  fetched_at: string | null;
  /** Total provider/model pairs in the cached catalog. */
  total: number;
  matches: CatalogMatch[];
}

export interface CatalogUpdateSummary {
  path: string;
  source_url: string;
  provider_count: number;
  model_count: number;
  fetched_at: string;
  bytes: number;
}

/** Download the latest models.dev catalog into the config directory. */
export function updateModelCatalog(sourceUrl?: string): Promise<CatalogUpdateSummary> {
  return transport.invoke<CatalogUpdateSummary>('model_catalog_update', {
    sourceUrl: sourceUrl ?? null,
  });
}

/** Fuzzy-search the cached catalog. An empty query browses it. */
export function searchModelCatalog(
  query: string,
  providerType?: string,
  limit = 50,
): Promise<CatalogSearchResult> {
  return transport.invoke<CatalogSearchResult>('model_catalog_search', {
    query,
    providerType: providerType ?? null,
    limit,
  });
}

/** Short human-readable summary of a catalog entry, e.g. "OpenAI - 128K ctx - tools". */
export function describeCatalogModel(model: CatalogModel): string {
  const parts = [model.provider_name];
  if (model.context_window) parts.push(`${formatTokens(model.context_window)} ctx`);
  if (model.tool_call) parts.push('tools');
  if (model.reasoning) parts.push('reasoning');
  if (model.capabilities.includes('vision')) parts.push('vision');
  return parts.join(' \u00b7 ');
}

/** Format a token count compactly: 128000 -> "128K", 1048576 -> "1M". */
export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  }
  if (tokens >= 1_000) {
    return `${(tokens / 1_000).toFixed(1).replace(/\.0$/, '')}K`;
  }
  return String(tokens);
}
