import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
}));

import { DiagnosticsPanel } from '../components/observation/DiagnosticsPanel';
import { computeSummary } from '../hooks/useDiagnostics';
import { ThemeContext } from '../hooks/useTheme';
import type { DiagnosticsEntry } from '../types';

describe('DiagnosticsPanel', () => {
  it('renders the producing agent name for diagnostics entries', () => {
    const entries: DiagnosticsEntry[] = [
      {
        id: 'diag-1',
        timestamp: new Date().toISOString(),
        event: {
          type: 'llm_response',
          iteration: 1,
          model: 'gpt-test',
          input_tokens: 120,
          output_tokens: 30,
          duration_ms: 40,
          cost_usd: 0.001,
          tool_calls_requested: [],
          prompt_preview: '{"messages":[]}',
          response_text: '{"content":"ok"}',
          context_window: 128000,
          agent_name: 'subagent:plan-writer',
        },
      },
      {
        id: 'diag-2',
        timestamp: new Date().toISOString(),
        event: {
          type: 'tool_result',
          name: 'FileRead',
          success: true,
          duration_ms: 12,
          input_preview: '{"path":"Cargo.toml"}',
          result_preview: '{"content":"[package]"}',
          agent_name: 'chat-turn',
        },
      },
    ];

    const html = renderToStaticMarkup(
      <ThemeContext.Provider value={{ resolvedTheme: 'dark' }}>
        <DiagnosticsPanel
          entries={entries}
          summary={computeSummary(entries)}
          isActive={false}
          isGlobal={false}
          sessionId="session-1"
          expanded={false}
          onToggleExpand={() => {}}
          onClear={() => {}}
          onClose={() => {}}
        />
      </ThemeContext.Provider>,
    );

    expect(html).toContain('subagent:plan-writer');
    expect(html).toContain('chat-turn');
  });
});
