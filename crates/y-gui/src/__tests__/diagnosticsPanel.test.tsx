import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ReactNode } from 'react';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(async () => []),
    listen: vi.fn(async () => () => {}),
  },
}));

vi.mock('../components/ui', async () => {
  const React = await import('react');
  return {
    Button: ({ children, ...props }: { children?: ReactNode } & Record<string, unknown>) =>
      React.createElement('button', props, children),
  };
});

import { DiagnosticsPanel } from '../components/observation/DiagnosticsPanel';
import {
  MAX_DIAGNOSTICS_PANEL_WIDTH,
  MIN_DIAGNOSTICS_PANEL_WIDTH,
  diagnosticsPanelWidthFromPointer,
} from '../components/observation/diagnosticsPanelResize';
import { clearPersistedDiagnostics, computeSummary } from '../hooks/useDiagnostics';
import { ThemeContext } from '../hooks/useTheme';
import { transport } from '../lib';
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

  it('exposes a resize separator when docked beside the main panel', () => {
    const html = renderToStaticMarkup(
      <ThemeContext.Provider value={{ resolvedTheme: 'dark' }}>
        <DiagnosticsPanel
          entries={[]}
          summary={computeSummary([])}
          isActive={false}
          isGlobal={false}
          sessionId={null}
          expanded={false}
          onToggleExpand={() => {}}
          onClear={() => {}}
          onClose={() => {}}
        />
      </ThemeContext.Provider>,
    );

    expect(html).toContain('role="separator"');
    expect(html).toContain('aria-label="Resize diagnostics panel"');
    expect(html).toContain('aria-orientation="vertical"');
  });

  it('maps divider drag position to a constrained diagnostics panel width', () => {
    expect(diagnosticsPanelWidthFromPointer({ clientX: 800, viewportWidth: 1200 })).toBe(400);
    expect(diagnosticsPanelWidthFromPointer({ clientX: 1100, viewportWidth: 1200 })).toBe(
      MIN_DIAGNOSTICS_PANEL_WIDTH,
    );
    expect(diagnosticsPanelWidthFromPointer({ clientX: 20, viewportWidth: 1440 })).toBe(
      MAX_DIAGNOSTICS_PANEL_WIDTH,
    );
  });

  it('deletes persisted diagnostics for the active session when clearing', async () => {
    const invoke = vi.mocked(transport.invoke);
    invoke.mockClear();

    await clearPersistedDiagnostics('session-1');

    expect(invoke).toHaveBeenCalledWith('diagnostics_clear_by_session', { sessionId: 'session-1' });
  });

  it('deletes all persisted diagnostics from the global view when clearing', async () => {
    const invoke = vi.mocked(transport.invoke);
    invoke.mockClear();

    await clearPersistedDiagnostics(null);

    expect(invoke).toHaveBeenCalledWith('diagnostics_clear_all');
  });
});
