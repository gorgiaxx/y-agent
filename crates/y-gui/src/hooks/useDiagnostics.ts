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
import { transport, type UnlistenFn } from '../lib';
import type {
  DiagnosticsEntry,
  DiagnosticsGatewayEvent,
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
    } else if (ev.type === 'llm_error') {
      s.totalIterations = Math.max(s.totalIterations, ev.iteration);
      s.totalDurationMs += ev.duration_ms;
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

const unlistenFns: UnlistenFn[] = [];

const NIL_UUID = '00000000-0000-0000-0000-000000000000';

const MAX_DIAG_ENTRIES_PER_SESSION = 200;
const DIAG_TRIM_TARGET = 100;

/**
 * Map a raw backend diagnostics record to a typed DiagnosticsEntry.
 * Shared by loadSubagentHistory, reloadSessionHistory, and the session
 * history loader inside the hook.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function mapRawToEntry(item: any, idPrefix: string, idx: number): DiagnosticsEntry {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let event: any;
  const timestamp = (item.timestamp as string) || new Date().toISOString();
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
        context_window: item.context_window as number,
        agent_name: item.agent_name as string | undefined,
      };
      break;
    case 'tool_result':
      event = {
        type: 'tool_result' as const,
        name: item.name as string,
        success: item.success as boolean,
        duration_ms: item.duration_ms as number,
        input_preview: (item.input_preview as string) ?? undefined,
        result_preview: item.result_preview as string,
        agent_name: item.agent_name as string | undefined,
      };
      break;
    default:
      event = { type: 'user_message' as const, content: '' };
  }
  return { id: `${idPrefix}-${idx}`, timestamp, event };
}

/** Fetch all subagent diagnostics (across all sessions) from the database and
 *  replace the nil-UUID entries in `sharedState`. Called on init and again
 *  whenever a `diagnostics:subagent_completed` event arrives.              */
async function loadSubagentHistory() {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = await transport.invoke<any[]>('diagnostics_get_subagent_history', { limit: 50 });
    if (!raw || raw.length === 0) return;

    const histEntries = raw.map((item, idx) => mapRawToEntry(item, 'subagent', idx));

    broadcastUpdate((prev) => ({
      ...prev,
      sessionEntries: {
        ...prev.sessionEntries,
        [NIL_UUID]: histEntries,
      },
    }));
  } catch (e) {
    console.warn('loadSubagentHistory failed:', e);
  }
}

/** Reload diagnostics for a specific session from the backend, merging
 *  any new subagent entries with existing live entries. This is called
 *  when a subagent completes so its diagnostic entries become visible
 *  in the session-level view. */
async function reloadSessionHistory(sessionId: string) {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = await transport.invoke<any[]>('diagnostics_get_by_session', { sessionId, limit: 50 });
    if (!raw || raw.length === 0) return;

    const histEntries = raw.map((item, idx) => mapRawToEntry(item, `hist-${sessionId}`, idx));

    broadcastUpdate((prev) => {
      const existing = prev.sessionEntries[sessionId] ?? [];
      const existingIds = new Set(existing.map((e) => e.id));

      // Build content-level fingerprints from existing live entries so we
      // can detect when a history entry duplicates a live entry (same
      // observation, different ID prefix like 'diag-' vs 'hist-').
      const existingFingerprints = new Set<string>();
      for (const e of existing) {
        const ev = e.event;
        if (ev.type === 'llm_response') {
          existingFingerprints.add(
            `llm:${ev.agent_name ?? 'unknown'}:${ev.iteration}:${ev.model}:${ev.input_tokens}:${ev.output_tokens}:${ev.tool_calls_requested.join(',')}`,
          );
        } else if (ev.type === 'tool_result') {
          existingFingerprints.add(
            `tool:${ev.agent_name ?? 'unknown'}:${ev.name}:${ev.success}:${ev.duration_ms}`,
          );
        } else if (ev.type === 'user_message') {
          existingFingerprints.add(`user:${ev.content}`);
        }
      }

      const newEntries = histEntries.filter((e) => {
        // Skip if same ID already exists.
        if (existingIds.has(e.id)) return false;
        // Skip if a live entry with matching content already exists.
        const ev = e.event;
        let fp = '';
        if (ev.type === 'llm_response') {
          fp = `llm:${ev.agent_name ?? 'unknown'}:${ev.iteration}:${ev.model}:${ev.input_tokens}:${ev.output_tokens}:${ev.tool_calls_requested.join(',')}`;
        } else if (ev.type === 'tool_result') {
          fp = `tool:${ev.agent_name ?? 'unknown'}:${ev.name}:${ev.success}:${ev.duration_ms}`;
        } else if (ev.type === 'user_message') {
          fp = `user:${ev.content}`;
        }
        if (fp && existingFingerprints.has(fp)) return false;
        return true;
      });
      if (newEntries.length === 0) return prev;

      const merged = [...existing, ...newEntries].sort((a, b) =>
        a.timestamp.localeCompare(b.timestamp),
      );
      return {
        ...prev,
        sessionEntries: { ...prev.sessionEntries, [sessionId]: merged },
      };
    });
  } catch (e) {
    console.warn('reloadSessionHistory failed:', e);
  }
}

async function initialiseBus() {
  if (busInitialised) return;
  busInitialised = true;

  const u0 = await transport.listen<ChatStartedPayload>('chat:started', (event) => {
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

  const u1 = await transport.listen<ProgressPayload>('chat:progress', (event) => {
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
      let existing = [...(prev.sessionEntries[sid] ?? []), entry];
      if (existing.length > MAX_DIAG_ENTRIES_PER_SESSION) {
        existing = existing.slice(existing.length - DIAG_TRIM_TARGET);
      }
      return {
        ...prev,
        counter,
        sessionEntries: { ...prev.sessionEntries, [sid]: existing },
      };
    });
  });
  unlistenFns.push(u1);

  const u2 = await transport.listen<ChatCompletePayload>('chat:complete', (event) => {
    const { run_id } = event.payload;
    broadcastUpdate((prev) => {
      const next = new Set(prev.activeRuns);
      next.delete(run_id);
      delete prev.runToSession[run_id];
      return { ...prev, activeRuns: next };
    });
  });
  unlistenFns.push(u2);

  const u3 = await transport.listen<ChatErrorPayload>('chat:error', (event) => {
    const { run_id } = event.payload;
    broadcastUpdate((prev) => {
      const next = new Set(prev.activeRuns);
      next.delete(run_id);
      delete prev.runToSession[run_id];
      return { ...prev, activeRuns: next };
    });
  });
  unlistenFns.push(u3);

  // Gateway broadcast events (from DiagnosticsProviderPool / DiagnosticsToolGateway
  // / DiagnosticsAgentDelegator). These provide real-time LLM call, tool call,
  // and subagent completion visibility for ALL agent executions without
  // per-caller wiring. LLM/tool events are routed to the Global (nil-UUID)
  // view; subagent_completed triggers a DB history reload.
  const u4 = await transport.listen<DiagnosticsGatewayEvent>('diagnostics:event', (event) => {
    const ev = event.payload;

    // SubagentCompleted: trigger DB history reload so the diagnostics panel
    // shows persisted entries (survives app restart).
    if (ev.type === 'subagent_completed') {
      loadSubagentHistory();
      if (ev.session_id) {
        reloadSessionHistory(ev.session_id);
      }
      return;
    }

    let diagEntry: DiagnosticsEntry | null = null;

    if (ev.type === 'llm_call_completed') {
      diagEntry = {
        id: `broadcast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        timestamp: new Date().toISOString(),
        event: {
          type: 'llm_response',
          iteration: ev.iteration,
          model: ev.model,
          input_tokens: ev.input_tokens,
          output_tokens: ev.output_tokens,
          duration_ms: ev.duration_ms,
          cost_usd: ev.cost_usd,
          tool_calls_requested: ev.tool_calls_requested,
          prompt_preview: ev.prompt_preview,
          response_text: ev.response_text,
          context_window: ev.context_window,
          agent_name: ev.agent_name,
        },
      };
    } else if (ev.type === 'llm_call_failed') {
      diagEntry = {
        id: `broadcast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        timestamp: new Date().toISOString(),
        event: {
          type: 'llm_error',
          iteration: ev.iteration,
          model: ev.model,
          error: ev.error,
          duration_ms: ev.duration_ms,
          prompt_preview: '',
          context_window: 0,
          agent_name: ev.agent_name,
        },
      };
    } else if (ev.type === 'tool_call_completed') {
      diagEntry = {
        id: `broadcast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        timestamp: new Date().toISOString(),
        event: {
          type: 'tool_result',
          name: ev.tool_name,
          success: ev.success,
          duration_ms: ev.duration_ms,
          input_preview: ev.input_preview,
          result_preview: ev.result_preview,
          agent_name: ev.agent_name,
        },
      };
    }

    if (!diagEntry) return;

    // Always route to Global view. Session-bound events are already
    // covered by chat:progress, so we avoid inserting duplicates.
    broadcastUpdate((prev) => {
      let existing = [...(prev.sessionEntries[NIL_UUID] ?? []), diagEntry!];
      if (existing.length > MAX_DIAG_ENTRIES_PER_SESSION) {
        existing = existing.slice(existing.length - DIAG_TRIM_TARGET);
      }
      return {
        ...prev,
        counter: prev.counter + 1,
        sessionEntries: { ...prev.sessionEntries, [NIL_UUID]: existing },
      };
    });
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
    // eslint-disable-next-line react-hooks/set-state-in-effect
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
        const raw = await transport.invoke<any[]>('diagnostics_get_by_session', { sessionId: sid, limit: 50 });
        if (!raw || raw.length === 0) return;

        const histEntries = raw.map((item, idx) => mapRawToEntry(item, `hist-${sid}`, idx));

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
      // Global clear: wipe all session entries and stale run mappings.
      broadcastUpdate((prev) => ({
        ...prev,
        sessionEntries: {},
        runToSession: {},
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
