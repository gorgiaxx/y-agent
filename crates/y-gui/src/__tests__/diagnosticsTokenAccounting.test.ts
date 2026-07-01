import { describe, expect, it } from 'vitest';

import {
  computeSummary,
  llmResponseFromBroadcast,
  mapRawToEntry,
  type RawDiagnosticsRecord,
} from '../hooks/useDiagnostics';
import type { DiagnosticsEntry, DiagLlmCallCompleted } from '../types';

function llmEntry(overrides: Partial<Record<string, unknown>>): DiagnosticsEntry {
  return {
    id: 'e',
    timestamp: new Date().toISOString(),
    event: {
      type: 'llm_response',
      iteration: 1,
      model: 'claude',
      input_tokens: 797,
      output_tokens: 38,
      duration_ms: 10,
      cost_usd: 0.01,
      tool_calls_requested: [],
      prompt_preview: '',
      response_text: '',
      context_window: 200000,
      ...overrides,
    },
  } as DiagnosticsEntry;
}

describe('diagnostics token accounting', () => {
  it('computeSummary includes cache tokens in context total', () => {
    const entries = [
      llmEntry({
        input_tokens: 797,
        output_tokens: 38,
        cache_read_tokens: 12000,
        cache_write_tokens: 2048,
        context_tokens_used: 797 + 12000 + 2048,
      }),
    ];
    const s = computeSummary(entries);
    expect(s.totalInputTokens).toBe(797); // fresh only, unchanged
    expect(s.totalCacheReadTokens).toBe(12000);
    expect(s.totalCacheWriteTokens).toBe(2048);
    expect(s.totalContextTokens).toBe(797 + 12000 + 2048);
  });

  it('computeSummary derives context total when field absent', () => {
    const entries = [
      llmEntry({
        input_tokens: 500,
        cache_read_tokens: 3000,
        cache_write_tokens: 0,
        context_tokens_used: undefined,
      }),
    ];
    const s = computeSummary(entries);
    expect(s.totalContextTokens).toBe(500 + 3000);
  });

  it('mapRawToEntry copies cache/context fields from history record', () => {
    const raw: RawDiagnosticsRecord = {
      type: 'llm_response',
      input_tokens: 797,
      output_tokens: 38,
      cache_read_tokens: 12000,
      cache_write_tokens: 2048,
      context_tokens_used: 14845,
    };
    const entry = mapRawToEntry(raw, 'hist', 0);
    expect(entry.event.type).toBe('llm_response');
    if (entry.event.type === 'llm_response') {
      expect(entry.event.cache_read_tokens).toBe(12000);
      expect(entry.event.cache_write_tokens).toBe(2048);
      expect(entry.event.context_tokens_used).toBe(14845);
    }
  });

  it('mapRawToEntry falls back context_tokens_used to fresh input for old rows', () => {
    const raw: RawDiagnosticsRecord = {
      type: 'llm_response',
      input_tokens: 797,
      output_tokens: 38,
    };
    const entry = mapRawToEntry(raw, 'hist', 0);
    if (entry.event.type === 'llm_response') {
      expect(entry.event.context_tokens_used).toBe(797);
    }
  });

  it('llmResponseFromBroadcast preserves cache/context accounting', () => {
    const ev: DiagLlmCallCompleted = {
      type: 'llm_call_completed',
      trace_id: 't',
      observation_id: 'o',
      session_id: null,
      agent_name: 'chat-turn',
      iteration: 2,
      model: 'claude',
      input_tokens: 797,
      output_tokens: 38,
      cache_read_tokens: 12000,
      cache_write_tokens: 2048,
      context_tokens_used: 14845,
      duration_ms: 100,
      cost_usd: 0.02,
      tool_calls_requested: [],
      prompt_preview: '',
      response_text: '',
      context_window: 200000,
    };
    const out = llmResponseFromBroadcast(ev);
    expect(out.cache_read_tokens).toBe(12000);
    expect(out.cache_write_tokens).toBe(2048);
    expect(out.context_tokens_used).toBe(14845);
  });
});
