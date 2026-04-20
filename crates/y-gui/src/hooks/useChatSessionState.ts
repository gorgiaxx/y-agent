// ---------------------------------------------------------------------------
// useChatSessionState -- per-session operation status, pending edit,
// context reset points, compact points, and session switch restoration.
//
// Extracted from useChat.ts.
// ---------------------------------------------------------------------------

import { useState, useCallback, useEffect, type Dispatch, type SetStateAction, type MutableRefObject } from 'react';
import { transport } from '../lib';
import type { RequestMode } from '../types';
import { chatBusState } from './chatBus';
import { getPendingRunIdForSession } from './chatRunState';
import { getCachedMessages } from './chatHelpers';
import type { ChatSharedRefs } from './chatSharedState';
import type { ChatOpStatus, CompactInfo, PendingEdit } from './useChat';
import type { ToolResultRecord } from './chatStreamTypes';

export interface UseChatSessionStateReturn {
  opStatus: ChatOpStatus;
  opStatusRef: MutableRefObject<ChatOpStatus>;
  setOp: (status: ChatOpStatus) => void;
  setOpForSession: (sessionId: string, status: ChatOpStatus) => void;
  pendingEdit: PendingEdit | null;
  setPendingEdit: Dispatch<SetStateAction<PendingEdit | null>>;
  contextResetPoints: number[];
  setContextResetPoints: Dispatch<SetStateAction<number[]>>;
  compactPoints: CompactInfo[];
  setCompactPoints: Dispatch<SetStateAction<CompactInfo[]>>;
  addContextReset: () => void;
  addCompactPoint: (info: Omit<CompactInfo, 'atIndex'>) => void;
  invalidateStaleContextResets: (sessionId: string, newMsgCount: number) => void;
  markSessionActivity: (sessionId: string, at?: number) => void;
  syncSessionRunUi: (sessionId: string | null) => string | null;
  getRequestModeFromMessage: (message: { metadata?: Record<string, unknown> } | undefined) => RequestMode;
}

export function useChatSessionState(
  activeSessionId: string | null,
  refs: ChatSharedRefs,
  setVisibleToolResults: Dispatch<SetStateAction<ToolResultRecord[]>>,
  setError: Dispatch<SetStateAction<string | null>>,
): UseChatSessionStateReturn {
  // Operation state machine -- per-session.
  const [opStatus, setOpStatus] = useState<ChatOpStatus>('idle');

  const setOp = useCallback((status: ChatOpStatus) => {
    refs.opStatusRef.current = status;
    setOpStatus(status);
    // Persist into the per-session map.
    const sid = refs.activeSessionIdRef.current;
    if (sid) {
      refs.opStatusMapRef.current.set(sid, status);
    }
  }, [refs.opStatusRef, refs.activeSessionIdRef, refs.opStatusMapRef]);

  /** Update opStatus for a specific session. Only touches visible state
   *  if that session is currently active; otherwise just updates the map. */
  const setOpForSession = useCallback((sessionId: string, status: ChatOpStatus) => {
    refs.opStatusMapRef.current.set(sessionId, status);
    if (sessionId === refs.activeSessionIdRef.current) {
      refs.opStatusRef.current = status;
      setOpStatus(status);
    }
  }, [refs.opStatusMapRef, refs.activeSessionIdRef, refs.opStatusRef]);

  // Pending edit state (exposed to InputArea for banner).
  const [pendingEdit, setPendingEdit] = useState<PendingEdit | null>(null);

  // Per-session context reset points.
  const [contextResetPoints, setContextResetPoints] = useState<number[]>([]);

  // Per-session compaction points.
  const [compactPoints, setCompactPoints] = useState<CompactInfo[]>([]);

  const markSessionActivity = useCallback(
    (sessionId: string, at: number = Date.now()) => {
      refs.sessionActivityRef.current.set(sessionId, at);
    },
    [refs.sessionActivityRef],
  );

  const syncSessionRunUi = useCallback(
    (sessionId: string | null) => {
      if (!sessionId) {
        return null;
      }

      const pendingRunId = getPendingRunIdForSession(chatBusState, sessionId);
      if (pendingRunId) {
        chatBusState.streamingSessions.add(sessionId);
      }

      return pendingRunId;
    },
    [],
  );

  const getRequestModeFromMessage = useCallback(
    (message: { metadata?: Record<string, unknown> } | undefined): RequestMode => {
      const mode = message?.metadata?.request_mode;
      return mode === 'image_generation' ? 'image_generation' : 'text_chat';
    },
    [],
  );

  // Session switch effect: restore per-session state.
  useEffect(() => {
    refs.activeSessionIdRef.current = activeSessionId;
    // Cancel edit mode when switching sessions.
    if (pendingEdit) {
      setPendingEdit(null);
    }
    // Restore the new session's opStatus.
    const pendingRunId = syncSessionRunUi(activeSessionId);
    let restoredOp = activeSessionId
      ? (refs.opStatusMapRef.current.get(activeSessionId) ?? 'idle')
      : 'idle';
    if (activeSessionId && pendingRunId && restoredOp === 'idle') {
      restoredOp = 'sending';
      refs.opStatusMapRef.current.set(activeSessionId, restoredOp);
    }
    refs.opStatusRef.current = restoredOp;
    setOpStatus(restoredOp);
    // Restore tool results for the new session.
    if (activeSessionId) {
      setVisibleToolResults(refs.toolResultsRef.current.get(activeSessionId) ?? []);
    } else {
      setVisibleToolResults([]);
    }
    // Clear error from the previous session.
    setError(null);
    // Restore context reset points for the new session.
    if (activeSessionId) {
      const cached = refs.contextResetMapRef.current.get(activeSessionId);
      if (cached) {
        setContextResetPoints(cached);
      } else {
        // Load from backend (persisted across restarts).
        transport
          .invoke<number | null>('session_get_context_reset', {
            sessionId: activeSessionId,
          })
          .then((idx) => {
            if (refs.activeSessionIdRef.current !== activeSessionId) return;
            if (idx != null) {
              const points = [idx];
              refs.contextResetMapRef.current.set(activeSessionId, points);
              setContextResetPoints(points);
            } else {
              setContextResetPoints([]);
            }
          })
          .catch((e) =>
            console.warn('[chat] failed to load context reset:', e),
          );
      }
    } else {
      setContextResetPoints([]);
    }
    // Restore compact points for the new session.
    if (activeSessionId) {
      setCompactPoints(refs.compactMapRef.current.get(activeSessionId) ?? []);
    } else {
      setCompactPoints([]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId, syncSessionRunUi]);

  // ------------------------------------------------------------------
  // invalidateStaleContextResets
  // ------------------------------------------------------------------

  const invalidateStaleContextResets = useCallback(
    (sessionId: string, newMsgCount: number) => {
      const existing = refs.contextResetMapRef.current.get(sessionId) ?? [];
      const kept = existing.filter((idx) => idx < newMsgCount);
      if (kept.length === existing.length) return; // nothing changed
      refs.contextResetMapRef.current.set(sessionId, kept);
      if (refs.activeSessionIdRef.current === sessionId) {
        setContextResetPoints(kept);
      }
      // Persist: store the latest kept point (or clear if none remain).
      const persistIdx = kept.length > 0 ? kept[kept.length - 1] : null;
      transport
        .invoke('session_set_context_reset', { sessionId, index: persistIdx })
        .catch((e) =>
          console.error('[chat] failed to clear stale context reset:', e),
        );
    },
    [refs.contextResetMapRef, refs.activeSessionIdRef],
  );

  // ------------------------------------------------------------------
  // addContextReset
  // ------------------------------------------------------------------

  const addContextReset = useCallback(() => {
    const sid = refs.activeSessionIdRef.current;
    if (!sid) return;

    const msgs = getCachedMessages(refs.sessionMessagesRef.current, sid);
    const idx = msgs.length;
    const existing = refs.contextResetMapRef.current.get(sid) ?? [];
    if (existing.length > 0 && existing[existing.length - 1] === idx) return;
    const updated = [...existing, idx];
    refs.contextResetMapRef.current.set(sid, updated);
    setContextResetPoints(updated);

    transport
      .invoke('session_set_context_reset', { sessionId: sid, index: idx })
      .catch((e) => console.error('[chat] failed to persist context reset:', e));
  }, [refs.activeSessionIdRef, refs.sessionMessagesRef, refs.contextResetMapRef]);

  // ------------------------------------------------------------------
  // addCompactPoint
  // ------------------------------------------------------------------

  const addCompactPoint = useCallback(
    (info: Omit<CompactInfo, 'atIndex'>) => {
      const sid = refs.activeSessionIdRef.current;
      if (!sid) return;

      const msgs = getCachedMessages(refs.sessionMessagesRef.current, sid);
      const entry: CompactInfo = { ...info, atIndex: msgs.length };
      const existing = refs.compactMapRef.current.get(sid) ?? [];
      const updated = [...existing, entry];
      refs.compactMapRef.current.set(sid, updated);
      setCompactPoints(updated);
    },
    [refs.activeSessionIdRef, refs.sessionMessagesRef, refs.compactMapRef],
  );

  return {
    opStatus,
    opStatusRef: refs.opStatusRef,
    setOp,
    setOpForSession,
    pendingEdit,
    setPendingEdit,
    contextResetPoints,
    setContextResetPoints,
    compactPoints,
    setCompactPoints,
    addContextReset,
    addCompactPoint,
    invalidateStaleContextResets,
    markSessionActivity,
    syncSessionRunUi,
    getRequestModeFromMessage,
  };
}
