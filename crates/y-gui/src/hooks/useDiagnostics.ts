// Hook for collecting real-time diagnostics from the chat service layer.
//
// Listener registration is managed by a module-level singleton so React
// StrictMode double-mount never creates duplicate Tauri event listeners.
// Each session has its own entry buffer; switching sessions shows that
// session's diagnostics.
//
// History strategy: attempt to load stored observations from the backend
// on session switch. If the backend returns nothing (e.g. InMemoryTraceStore
// was reset on restart) we leave the buffer empty; live events populate it
// on the next run.

import { useState, useCallback, useEffect, useRef, useMemo } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type {
  DiagnosticsEntry,
  ProgressPayload,
  ChatCompletePayload,
  ChatErrorPayload,
  UserMessageEvent,
  ChatStartedPayload,
} from '../types';

export interface DiagnosticsSummary {
  totalIterations: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCost: number;
  totalDurationMs: number;
  toolCallCount: number;
  toolSuccessCount: number;
  toolFailCount: number;
}

interface UseDiagnosticsReturn {
  entries: DiagnosticsEntry[];
  summary: DiagnosticsSummary;
  isActive: boolean;
  clear: () => void;
  addUserMessage: (content: string, sessionId: string) => void;
}

const emptySummary: DiagnosticsSummary = {
  totalIterations: 0,
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCost: 0,
  totalDurationMs: 0,
  toolCallCount: 0,
  toolSuccessCount: 0,
  toolFailCount: 0,
};

export function computeSummary(entries: DiagnosticsEntry[]): DiagnosticsSummary {
  const s = { ...emptySummary };
  for (const e of entries) {
    const ev = e.event;
    if (ev.type === 'llm_response') {
      s.totalIterations = Math.max(s.totalIterations, ev.iteration);
      s.totalInputTokens += ev.input_tokens;
      s.totalOutputTokens += ev.output_tokens;
      s.totalCost += ev.cost_usd;
      s.totalDurationMs += ev.duration_ms;
    } else if (ev.type === 'tool_result') {
      s.toolCallCount += 1;
      s.totalDurationMs += ev.duration_ms;
      if (ev.success) s.toolSuccessCount += 1;
      else s.toolFailCount += 1;
    } else if (ev.type === 'loop_limit_hit') {
      s.totalIterations = ev.iterations;
    }
  }
  return s;
}

// ---------------------------------------------------------------------------
// Module-level event bus singleton
//
// Tauri event listeners are registered ONCE per application lifetime.
// State mutations are forwarded to all subscribed hook instances via
// a simple callback registry.  React StrictMode may mount/unmount the hook
// multiple times but the Tauri listeners are unaffected.
// ---------------------------------------------------------------------------

type StateUpdate = (updater: (prev: DiagnosticsState) => DiagnosticsState) => void;

interface DiagnosticsState {
  sessionEntries: Record<string, DiagnosticsEntry[]>;
  runToSession: Record<string, string>;
  activeRuns: Set<string>;
  counter: number;
}

let busInitialised = false;
const subscribers = new Set<StateUpdate>();
let sharedState: DiagnosticsState = {
  sessionEntries: {},
  runToSession: {},
  activeRuns: new Set(),
  counter: 0,
};

function broadcastUpdate(updater: (prev: DiagnosticsState) => DiagnosticsState) {
  sharedState = updater(sharedState);
  for (const cb of subscribers) {
    cb(updater);
  }
}

let unlistenFns: UnlistenFn[] = [];

const NIL_UUID = '00000000-0000-0000-0000-000000000000';

/** Fetch subagent diagnostics (stored under nil UUID) from the database and
 *  replace the nil-UUID entries in `sharedState`. Called on init and again
 *  whenever a `diagnostics:subagent_completed` event arrives.              */
async function loadSubagentHistory() {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = await invoke<any[]>('diagnostics_get_by_session', { sessionId: NIL_UUID, limit: 50 });
    if (!raw || raw.length === 0) return;

    const histEntries: DiagnosticsEntry[] = raw.map((item, idx) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      let event: any;
      const timestamp = item.timestamp as string;
      switch (item.type) {
        case 'llm_response':
          event = {
            type: 'llm_response' as const,
            iteration: item.iteration as number,
            model: item.model as string,
            input_tokens: item.input_tokens as number,
            output_tokens: item.output_tokens as number,
            duration_ms: item.duration_ms as number,
            cost_usd: item.cost_usd as number,
            tool_calls_requested: (item.tool_calls_requested ?? []) as string[],
            prompt_preview: item.prompt_preview as string,
            response_text: item.response_text as string,
          };
          break;
        case 'tool_result':
          event = {
            type: 'tool_result' as const,
            name: item.name as string,
            success: item.success as boolean,
            duration_ms: item.duration_ms as number,
            result_preview: item.result_preview as string,
          };
          break;
        default:
          event = { type: 'user_message' as const, content: '' };
      }
      return { id: `subagent-${idx}`, timestamp, event };
    });

    broadcastUpdate((prev) => ({
      ...prev,
      sessionEntries: {
        ...prev.sessionEntries,
        [NIL_UUID]: histEntries,
      },
    }));
  } catch {
    // Ignore -- no subagent history available.
  }
}

async function initialiseBus() {
  if (busInitialised) return;
  busInitialised = true;

  const u0 = await listen<ChatStartedPayload>('chat:started', (event) => {
    const { run_id, session_id } = event.payload;
    broadcastUpdate((prev) => {
      const next: DiagnosticsState = {
        ...prev,
        runToSession: { ...prev.runToSession, [run_id]: session_id },
        activeRuns: new Set([...prev.activeRuns, run_id]),
      };
      return next;
    });
  });
  unlistenFns.push(u0);

  const u1 = await listen<ProgressPayload>('chat:progress', (event) => {
    const { run_id, event: turnEvent } = event.payload;
    broadcastUpdate((prev) => {
      const sid = prev.runToSession[run_id];
      if (!sid) return prev; // unknown run -- ignore
      const counter = prev.counter + 1;
      const entry: DiagnosticsEntry = {
        id: `diag-${run_id}-${counter}`,
        timestamp: new Date().toISOString(),
        event: turnEvent,
      };
      return {
        ...prev,
        counter,
        sessionEntries: {
          ...prev.sessionEntries,
          [sid]: [...(prev.sessionEntries[sid] ?? []), entry],
        },
      };
    });
  });
  unlistenFns.push(u1);

  const u2 = await listen<ChatCompletePayload>('chat:complete', (event) => {
    const { run_id } = event.payload;
    broadcastUpdate((prev) => {
      const next = new Set(prev.activeRuns);
      next.delete(run_id);
      return { ...prev, activeRuns: next };
    });
  });
  unlistenFns.push(u2);

  const u3 = await listen<ChatErrorPayload>('chat:error', (event) => {
    const { run_id } = event.payload;
    broadcastUpdate((prev) => {
      const next = new Set(prev.activeRuns);
      next.delete(run_id);
      return { ...prev, activeRuns: next };
    });
  });
  unlistenFns.push(u3);

  // When a subagent (title-generator, skill-ingestion, etc.) completes,
  // the backend emits this event.  Re-fetch subagent history so the
  // Global diagnostics view picks up the new entries.
  const u4 = await listen('diagnostics:subagent_completed', () => {
    loadSubagentHistory();
  });
  unlistenFns.push(u4);

  // Seed subagent history on first load.
  loadSubagentHistory();
}

// Kick off bus initialisation immediately (not inside any hook) so the first
// event can never be missed due to hook mount timing.
initialiseBus().catch(console.error);

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useDiagnostics(activeSessionId: string | null): UseDiagnosticsReturn {
  // Local mirror of the shared state -- only used for React re-renders.
  const [localState, setLocalState] = useState<DiagnosticsState>(sharedState);

  // Track which session we last attempted a history load for.
  const historyLoadedFor = useRef<string | null>(null);

  // Subscribe to the module-level bus on mount, unsubscribe on unmount.
  useEffect(() => {
    // Sync immediately in case sharedState was updated before mount.
    setLocalState(sharedState);

    const update: StateUpdate = (updater) => {
      setLocalState(updater);
    };
    subscribers.add(update);
    return () => {
      subscribers.delete(update);
    };
  }, []);

  // Load history from the backend when the active session changes,
  // but only if no live entries are already present.
  useEffect(() => {
    if (!activeSessionId) return;
    if (historyLoadedFor.current === activeSessionId) return;
    historyLoadedFor.current = activeSessionId;

    const sid = activeSessionId;
    (async () => {
      // Do not overwrite live entries.
      if ((sharedState.sessionEntries[sid] ?? []).length > 0) return;

      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const raw = await invoke<any[]>('diagnostics_get_by_session', { sessionId: sid, limit: 50 });
        if (!raw || raw.length === 0) return;

        const histEntries: DiagnosticsEntry[] = raw.map((item, idx) => {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          let event: any;
          const timestamp = item.timestamp as string;
          switch (item.type) {
            case 'user_message':
              event = { type: 'user_message' as const, content: item.content as string };
              break;
            case 'llm_response':
              event = {
                type: 'llm_response' as const,
                iteration: item.iteration as number,
                model: item.model as string,
                input_tokens: item.input_tokens as number,
                output_tokens: item.output_tokens as number,
                duration_ms: item.duration_ms as number,
                cost_usd: item.cost_usd as number,
                tool_calls_requested: (item.tool_calls_requested ?? []) as string[],
                prompt_preview: item.prompt_preview as string,
                response_text: item.response_text as string,
              };
              break;
            case 'tool_result':
              event = {
                type: 'tool_result' as const,
                name: item.name as string,
                success: item.success as boolean,
                duration_ms: item.duration_ms as number,
                result_preview: item.result_preview as string,
              };
              break;
            default:
              event = { type: 'user_message' as const, content: '' };
          }
          return { id: `hist-${sid}-${idx}`, timestamp, event };
        });

        // Final guard: only seed if still no live entries.
        broadcastUpdate((prev) => {
          if ((prev.sessionEntries[sid] ?? []).length > 0) return prev;
          return {
            ...prev,
            sessionEntries: { ...prev.sessionEntries, [sid]: histEntries },
          };
        });
      } catch (err) {
        // Backend has no stored diagnostics (e.g. InMemoryStore after restart) -- ignore.
        console.debug('diagnostics_get_by_session failed:', err);
      }
    })();
  }, [activeSessionId]);


  const clear = useCallback(() => {
    if (activeSessionId) {
      const sid = activeSessionId;
      broadcastUpdate((prev) => ({
        ...prev,
        sessionEntries: { ...prev.sessionEntries, [sid]: [] },
      }));
    } else {
      // Global clear: wipe all session entries.
      broadcastUpdate((prev) => ({
        ...prev,
        sessionEntries: {},
      }));
    }
  }, [activeSessionId]);

  const addUserMessage = useCallback((content: string, sessionId: string) => {
    const event: UserMessageEvent = { type: 'user_message', content };
    broadcastUpdate((prev) => {
      const counter = prev.counter + 1;
      const entry: DiagnosticsEntry = {
        id: `diag-user-${counter}`,
        timestamp: new Date().toISOString(),
        event,
      };
      return {
        ...prev,
        counter,
        sessionEntries: {
          ...prev.sessionEntries,
          [sessionId]: [...(prev.sessionEntries[sessionId] ?? []), entry],
        },
      };
    });
  }, []);

  const entries = useMemo(() => {
    if (activeSessionId) {
      return localState.sessionEntries[activeSessionId] ?? [];
    }
    // Global view: merge all sessions' entries sorted by timestamp.
    const all = Object.values(localState.sessionEntries).flat();
    return all.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
  }, [activeSessionId, localState]);
  const isActive = activeSessionId
    ? [...localState.activeRuns].some((rid) => localState.runToSession[rid] === activeSessionId)
    : localState.activeRuns.size > 0;
  const summary = useMemo(() => computeSummary(entries), [entries]);

  return { entries, summary, isActive, clear, addUserMessage };
}
