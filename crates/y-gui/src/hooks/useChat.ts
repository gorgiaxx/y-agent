// Custom hook for chat functionality -- per-session streaming state.
//
// Architecture (post-refactoring):
// - Module-level ChatBus singleton handles Tauri event listeners.
// - Per-session message cache survives session switches.
// - Operation state machine prevents illegal concurrent operations.
// - Session lock serialises compound operations (edit, undo, resend).
// - All compound operations are transactional: backend-first, then UI.

import { useState, useCallback, useEffect, useRef, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  Message,
  ChatStarted,
  ChatCompletePayload,
  ChatErrorPayload,
  ChatStartedPayload,
  ChatCheckpointInfo,
  ProgressPayload,
  UndoResult,
  RestoreResult,
} from '../types';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Pending edit state exposed to InputArea via App.tsx. */
export interface PendingEdit {
  messageId: string;
  content: string;
}

/** Operation status for guarding concurrent actions. */
export type ChatOpStatus =
  | 'idle'
  | 'sending'
  | 'editing'
  | 'undoing'
  | 'resending'
  | 'restoring';

interface UseChatReturn {
  messages: Message[];
  isStreaming: boolean;
  isLoadingMessages: boolean;
  streamingSessionIds: Set<string>;
  activeRunId: string | null;
  error: string | null;
  /** Current high-level operation status. */
  opStatus: ChatOpStatus;
  /** Pending edit info (for InputArea banner). */
  pendingEdit: PendingEdit | null;
  sendMessage: (message: string, sessionId: string, providerId?: string) => Promise<ChatStarted | null>;
  cancelRun: () => Promise<void>;
  loadMessages: (sessionId: string) => Promise<void>;
  clearMessages: () => void;
  /** Enter edit mode: populate input box, show edit banner.
   *  No optimistic truncation -- the UI stays unchanged until send. */
  editMessage: (messageId: string, content: string) => void;
  /** Cancel an in-progress edit (restore original view). */
  cancelEdit: () => void;
  /** Execute edit and resend: undo to checkpoint then send new content.
   *  This is the transactional compound operation called from handleSend. */
  editAndResend: (sessionId: string, newContent: string, providerId?: string) => Promise<ChatStarted | null>;
  /** Undo to a specific message: rolls back all state to before that message was sent. */
  undoToMessage: (sessionId: string, messageId: string) => Promise<UndoResult | null>;
  /** Resend: keep user message, remove assistant reply, re-run LLM. */
  resendLastTurn: (sessionId: string, messageId: string, content: string, providerId?: string) => Promise<ChatStarted | null>;
  /** Restore a tombstoned branch. */
  restoreBranch: (sessionId: string, checkpointId: string) => Promise<RestoreResult | null>;
}

// ---------------------------------------------------------------------------
// Module-level singleton bus (unchanged from original)
// ---------------------------------------------------------------------------

interface ChatBusState {
  runToSession: Record<string, string>;
  streamingSessions: Set<string>;
  pendingRuns: Set<string>;
}

type ChatBusSubscriber = (event: ChatBusEvent) => void;

type ChatBusEvent =
  | { type: 'started'; run_id: string; session_id: string }
  | { type: 'complete'; payload: ChatCompletePayload }
  | { type: 'error'; payload: ChatErrorPayload }
  | { type: 'stream_delta'; run_id: string; session_id: string; content: string };

let chatBusInitialised = false;
const chatBusState: ChatBusState = {
  runToSession: {},
  streamingSessions: new Set(),
  pendingRuns: new Set(),
};
const chatBusSubscribers = new Set<ChatBusSubscriber>();
const chatUnlistenFns: UnlistenFn[] = [];

function notifyChatSubscribers(event: ChatBusEvent) {
  for (const cb of chatBusSubscribers) {
    cb(event);
  }
}

async function initialiseChatBus() {
  if (chatBusInitialised) return;
  chatBusInitialised = true;

  const u0 = await listen<ChatStartedPayload>('chat:started', (e) => {
    const { run_id, session_id } = e.payload;
    chatBusState.runToSession[run_id] = session_id;
    chatBusState.pendingRuns.add(run_id);
    chatBusState.streamingSessions.add(session_id);
    notifyChatSubscribers({ type: 'started', run_id, session_id });
  });
  chatUnlistenFns.push(u0);

  const u1 = await listen<ChatCompletePayload>('chat:complete', (e) => {
    const { run_id } = e.payload;
    const session_id = chatBusState.runToSession[run_id];
    chatBusState.pendingRuns.delete(run_id);
    if (session_id) chatBusState.streamingSessions.delete(session_id);
    notifyChatSubscribers({ type: 'complete', payload: e.payload });
  });
  chatUnlistenFns.push(u1);

  const u2 = await listen<ChatErrorPayload>('chat:error', (e) => {
    const { run_id } = e.payload;
    const session_id = chatBusState.runToSession[run_id];
    chatBusState.pendingRuns.delete(run_id);
    if (session_id) chatBusState.streamingSessions.delete(session_id);
    notifyChatSubscribers({ type: 'error', payload: e.payload });
  });
  chatUnlistenFns.push(u2);

  const u3 = await listen<ProgressPayload>('chat:progress', (e) => {
    const { run_id, event } = e.payload;
    if (event.type === 'stream_delta') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'stream_delta',
          run_id,
          session_id,
          content: event.content,
        });
      }
    }
  });
  chatUnlistenFns.push(u3);
}

// Kick off immediately so events are never missed due to mount timing.
initialiseChatBus().catch(console.error);

// ---------------------------------------------------------------------------
// Per-session message cache helpers
// ---------------------------------------------------------------------------

function getCachedMessages(
  cache: Map<string, Message[]>,
  sessionId: string,
): Message[] {
  return cache.get(sessionId) ?? [];
}

function setCachedMessages(
  cache: Map<string, Message[]>,
  sessionId: string,
  updater: Message[] | ((prev: Message[]) => Message[]),
): Message[] {
  const prev = cache.get(sessionId) ?? [];
  const next = typeof updater === 'function' ? updater(prev) : updater;
  cache.set(sessionId, next);
  return next;
}

// ---------------------------------------------------------------------------
// Session lock -- serialises compound operations per session
// ---------------------------------------------------------------------------

const sessionLocks = new Map<string, Promise<void>>();

async function withSessionLock<T>(sessionId: string, fn: () => Promise<T>): Promise<T> {
  const prev = sessionLocks.get(sessionId) ?? Promise.resolve();
  let resolve: () => void;
  const next = new Promise<void>((r) => { resolve = r; });
  sessionLocks.set(sessionId, next);

  // Wait for previous operation to complete.
  await prev;

  try {
    return await fn();
  } finally {
    resolve!();
  }
}

// ---------------------------------------------------------------------------
// Checkpoint resolution helpers
// ---------------------------------------------------------------------------

/** Find the checkpoint for a specific user message by its ID.
 *  We list all checkpoints and find the one whose message_count_before
 *  matches the message's position in the backend-persisted messages.
 *
 *  Uses content+role matching as the primary strategy (robust against
 *  ID drift between optimistic/streaming IDs and backend UUIDs), with
 *  exact ID match as a fast path.
 *
 *  Falls back to the most recent checkpoint if no match is found. */
async function findCheckpointForMessage(
  sessionId: string,
  messageId: string,
  cache?: Map<string, Message[]>,
): Promise<ChatCheckpointInfo | null> {
  // Load messages from backend to get the canonical order.
  const backendMessages = await invoke<Message[]>('session_get_messages', { sessionId });
  console.log(`[chat] findCheckpointForMessage: backend has ${backendMessages.length} messages, looking for id=${messageId}`);

  // Fast path: exact ID match.
  let messageIndex = backendMessages.findIndex((m) => m.id === messageId);

  // Primary strategy: content+role match (robust against ID drift).
  // This covers optimistic IDs (`user-{timestamp}`), streaming IDs, and
  // stale IDs from a previous render cycle.
  if (messageIndex < 0) {
    // Try to find the message content either from cache or from backendMessages.
    let targetContent: string | null = null;
    let targetRole: string | null = null;

    if (cache) {
      const cachedMessages = cache.get(sessionId) ?? [];
      const cachedMsg = cachedMessages.find((m) => m.id === messageId);
      if (cachedMsg) {
        targetContent = cachedMsg.content;
        targetRole = cachedMsg.role;
      }
    }

    if (targetContent !== null && targetRole !== null) {
      messageIndex = backendMessages.findIndex(
        (m) => m.role === targetRole && m.content === targetContent,
      );
      console.log(`[chat] findCheckpointForMessage: content match found at index=${messageIndex}`);
    }
  }

  const checkpoints = await invoke<ChatCheckpointInfo[]>('chat_checkpoint_list', {
    sessionId,
  });

  console.log(`[chat] findCheckpointForMessage: ${checkpoints.length} checkpoints, messageIndex=${messageIndex}`);

  if (checkpoints.length === 0) return null;

  // Find checkpoint whose message_count_before matches this message's index.
  if (messageIndex >= 0) {
    const exactMatch = checkpoints.find(
      (cp) => cp.message_count_before === messageIndex,
    );
    if (exactMatch) return exactMatch;
  }

  // No match found -- do NOT fallback to an arbitrary checkpoint, as that
  // would truncate to the wrong point and delete the user's messages.
  console.warn(`[chat] findCheckpointForMessage: no checkpoint matched messageIndex=${messageIndex}, available:`, checkpoints.map(cp => `turn=${cp.turn_number} msg_before=${cp.message_count_before}`));
  return null;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useChat(activeSessionId: string | null): UseChatReturn {
  // Per-session message cache -- survives session switches.
  const sessionMessagesRef = useRef(new Map<string, Message[]>());

  // The messages that the UI actually renders (derived from the active session).
  const [visibleMessages, setVisibleMessages] = useState<Message[]>([]);

  const [streamingSessionIds, setStreamingSessionIds] = useState<Set<string>>(
    new Set(chatBusState.streamingSessions),
  );
  const [error, setError] = useState<string | null>(null);
  const [isLoadingMessages, setIsLoadingMessages] = useState(false);
  const activeRunIdRef = useRef<string | null>(null);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);

  // Operation state machine.
  const [opStatus, setOpStatus] = useState<ChatOpStatus>('idle');
  const opStatusRef = useRef<ChatOpStatus>('idle');
  const setOp = useCallback((status: ChatOpStatus) => {
    opStatusRef.current = status;
    setOpStatus(status);
  }, []);

  // Pending edit state (exposed to InputArea for banner).
  const [pendingEdit, setPendingEdit] = useState<PendingEdit | null>(null);

  // Keep a ref in sync with activeSessionId.
  const activeSessionIdRef = useRef<string | null>(activeSessionId);
  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
    // Cancel edit mode when switching sessions.
    if (pendingEdit) {
      setPendingEdit(null);
      setOp('idle');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId]);

  // Flush visible messages from cache for the given session.
  const syncVisible = useCallback((sessionId: string) => {
    if (sessionId === activeSessionIdRef.current) {
      setVisibleMessages(
        getCachedMessages(sessionMessagesRef.current, sessionId),
      );
    }
  }, []);

  // Subscribe to the chat bus on mount.
  useEffect(() => {
    setStreamingSessionIds(new Set(chatBusState.streamingSessions));

    const handler: ChatBusSubscriber = (event) => {
      if (event.type === 'started') {
        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        activeRunIdRef.current = event.run_id;
        setActiveRunId(event.run_id);
        console.log('[chat] run started, run_id =', event.run_id, 'session =', event.session_id);
      } else if (event.type === 'stream_delta') {
        const sid = event.session_id;
        setCachedMessages(sessionMessagesRef.current, sid, (prev) => {
          const streamingId = `streaming-${sid}`;
          const lastIdx = prev.findIndex((m) => m.id === streamingId);
          if (lastIdx >= 0) {
            const updated = [...prev];
            updated[lastIdx] = {
              ...updated[lastIdx],
              content: updated[lastIdx].content + event.content,
            };
            return updated;
          }
          return [
            ...prev,
            {
              id: streamingId,
              role: 'assistant' as const,
              content: event.content,
              timestamp: new Date().toISOString(),
              tool_calls: [],
              _streaming: true,
            } as Message,
          ];
        });
        syncVisible(sid);
      } else if (event.type === 'complete') {
        const payload = event.payload;
        const sessionId = chatBusState.runToSession[payload.run_id];
        console.log(`[chat] complete: run_id=${payload.run_id}, session=${sessionId}, opStatus=${opStatusRef.current}`);

        if (sessionId) {
          // Reload from backend to get messages with real backend IDs.
          // CRITICAL: we must await this reload and update the cache/visible
          // BEFORE transitioning opStatus to idle, so that subsequent
          // operations (resend, edit) see consistent message IDs.
          (async () => {
            try {
              const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
              setCachedMessages(sessionMessagesRef.current, sessionId, msgs);
              if (activeSessionIdRef.current === sessionId) {
                startTransition(() => {
                  setVisibleMessages(msgs);
                });
              }
            } catch (e) {
              console.error('[chat] complete: failed to reload messages:', e);
              // Fallback: synthesize the assistant message in cache.
              setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => {
                const streamingId = `streaming-${sessionId}`;
                const filtered = prev.filter((m) => m.id !== streamingId);
                const newMsg: Message = {
                  id: `assistant-${payload.run_id}`,
                  role: 'assistant' as const,
                  content: payload.content,
                  timestamp: new Date().toISOString(),
                  tool_calls: payload.tool_calls.map((tc) => ({
                    id: tc.name,
                    name: tc.name,
                    arguments: '',
                  })),
                  model: payload.model,
                  provider_id: payload.provider_id,
                  tokens: { input: payload.input_tokens, output: payload.output_tokens },
                  cost: payload.cost_usd,
                  context_window: payload.context_window,
                };
                if (filtered.some((m) => m.id === newMsg.id)) return filtered;
                return [...filtered, newMsg];
              });
              syncVisible(sessionId);
            } finally {
              // Transition to idle AFTER the cache is updated, not before.
              if (opStatusRef.current !== 'idle') {
                setOp('idle');
              }
            }
          })();
        } else {
          // No session to reload -- transition immediately.
          if (opStatusRef.current !== 'idle') {
            setOp('idle');
          }
        }

        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        if (activeRunIdRef.current === payload.run_id) {
          activeRunIdRef.current = null;
          setActiveRunId(null);
        }
        setError(null);
      } else if (event.type === 'error') {
        const payload = event.payload;
        const sessionId = chatBusState.runToSession[payload.run_id];
        const isCancelled = payload.error === 'Cancelled';

        if (sessionId) {
          if (isCancelled) {
            // Stop/cancel: preserve any streamed content by finalizing the
            // streaming message instead of deleting it. This keeps the
            // partially-streamed text visible and treats the run as complete.
            setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => {
              const streamingId = `streaming-${sessionId}`;
              return prev.map((m) => {
                if (m.id === streamingId && m.content) {
                  return {
                    ...m,
                    id: `cancelled-${payload.run_id}`,
                    _streaming: undefined,
                  } as Message;
                }
                // If streaming message has no content, remove it.
                if (m.id === streamingId) return null;
                return m;
              }).filter(Boolean) as Message[];
            });
          } else {
            // Non-cancel error: remove the streaming message entirely.
            setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => {
              const streamingId = `streaming-${sessionId}`;
              return prev.filter((m) => m.id !== streamingId);
            });
          }
          syncVisible(sessionId);
        }

        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        if (activeRunIdRef.current === payload.run_id) {
          activeRunIdRef.current = null;
          setActiveRunId(null);
        }
        if (!isCancelled) {
          if (!sessionId || sessionId === activeSessionIdRef.current) {
            setError(payload.error);
          }
        }

        // Return to idle on error too.
        if (opStatusRef.current !== 'idle') {
          setOp('idle');
        }
      }
    };

    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  }, [syncVisible, setOp]);

  // ------------------------------------------------------------------
  // Core operations
  // ------------------------------------------------------------------

  const loadMessages = useCallback(async (sessionId: string) => {
    activeSessionIdRef.current = sessionId;

    startTransition(() => {
      setVisibleMessages(getCachedMessages(sessionMessagesRef.current, sessionId));
      setIsLoadingMessages(true);
    });

    try {
      const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
      console.log(`[chat] loadMessages: got ${msgs.length} messages for session=${sessionId}, active=${activeSessionIdRef.current}`);
      if (activeSessionIdRef.current === sessionId) {
        const streamingId = `streaming-${sessionId}`;
        const existingStreaming = getCachedMessages(
          sessionMessagesRef.current,
          sessionId,
        ).find((m) => m.id === streamingId);

        const merged = existingStreaming ? [...msgs, existingStreaming] : msgs;
        setCachedMessages(sessionMessagesRef.current, sessionId, merged);
        startTransition(() => {
          setVisibleMessages(merged);
        });
      } else {
        console.log(`[chat] loadMessages: session mismatch, skipping visible update (active=${activeSessionIdRef.current}, requested=${sessionId})`);
        setCachedMessages(sessionMessagesRef.current, sessionId, msgs);
      }
    } catch (e) {
      console.error('[chat] loadMessages failed:', e);
      setError(String(e));
    } finally {
      if (activeSessionIdRef.current === sessionId) {
        startTransition(() => {
          setIsLoadingMessages(false);
        });
      }
    }
  }, []);

  const clearMessages = useCallback(() => {
    const sid = activeSessionIdRef.current;
    if (sid) {
      sessionMessagesRef.current.delete(sid);
    }
    activeSessionIdRef.current = null;
    setVisibleMessages([]);
    setError(null);
    setPendingEdit(null);
    setOp('idle');
  }, [setOp]);

  const isStreaming = activeSessionId ? streamingSessionIds.has(activeSessionId) : false;

  const cancelRun = useCallback(async () => {
    let runId = activeRunIdRef.current;
    if (!runId) {
      const sessionId = activeSessionIdRef.current;
      if (sessionId) {
        runId = [...chatBusState.pendingRuns].find(
          (rid) => chatBusState.runToSession[rid] === sessionId,
        ) ?? null;
      }
    }
    if (!runId) {
      console.warn('[chat] cancelRun: no active run found');
      return;
    }
    try {
      await invoke('chat_cancel', { runId });
    } catch (e) {
      console.error('[chat] chat_cancel failed:', e);
    }
  }, []);

  // ------------------------------------------------------------------
  // sendMessage -- raw send with optimistic UI
  // ------------------------------------------------------------------

  const sendMessage = useCallback(
    async (message: string, sessionId: string, providerId?: string): Promise<ChatStarted | null> => {
      // Guard: block if a compound operation is in progress.
      if (opStatusRef.current !== 'idle' && opStatusRef.current !== 'sending') {
        console.warn(`[chat] sendMessage blocked: opStatus=${opStatusRef.current}`);
        return null;
      }

      setError(null);
      setOp('sending');
      activeSessionIdRef.current = sessionId;

      // Optimistic user message.
      const userMsg: Message = {
        id: `user-${Date.now()}`,
        role: 'user' as const,
        content: message,
        timestamp: new Date().toISOString(),
        tool_calls: [],
      };
      setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => [...prev, userMsg]);
      syncVisible(sessionId);

      try {
        const result = await invoke<ChatStarted>('chat_send', {
          message,
          sessionId,
          providerId: providerId ?? null,
        });
        return result;
      } catch (e) {
        setError(String(e));
        chatBusState.streamingSessions.delete(sessionId);
        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        setOp('idle');

        // Rollback: remove the optimistic message on failure.
        setCachedMessages(sessionMessagesRef.current, sessionId, (prev) =>
          prev.filter((m) => m.id !== userMsg.id),
        );
        syncVisible(sessionId);
        return null;
      }
    },
    [syncVisible, setOp],
  );

  // ------------------------------------------------------------------
  // editMessage -- enter edit mode (no truncation, no backend call)
  // ------------------------------------------------------------------

  const editMessage = useCallback((messageId: string, content: string) => {
    setPendingEdit({ messageId, content });
  }, []);

  // ------------------------------------------------------------------
  // cancelEdit -- exit edit mode, no changes
  // ------------------------------------------------------------------

  const cancelEdit = useCallback(() => {
    setPendingEdit(null);
  }, []);

  // ------------------------------------------------------------------
  // editAndResend -- transactional: undo to checkpoint, then send
  // ------------------------------------------------------------------

  const editAndResend = useCallback(
    async (sessionId: string, newContent: string, providerId?: string): Promise<ChatStarted | null> => {
      if (opStatusRef.current !== 'idle') {
        console.warn(`[chat] editAndResend blocked: opStatus=${opStatusRef.current}`);
        return null;
      }

      const edit = pendingEdit;
      if (!edit) {
        console.warn('[chat] editAndResend called without pending edit');
        return null;
      }

      setOp('editing');
      setError(null);

      return withSessionLock(sessionId, async () => {
        try {
          // 1. Find checkpoint for the edited message (backend query, not cache).
          const checkpoint = await findCheckpointForMessage(sessionId, edit.messageId, sessionMessagesRef.current);
          if (!checkpoint) {
            console.warn('[chat] No checkpoint found for edit; aborting');
            setOp('idle');
            return null;
          }

          // 2. Backend undo -- truncate transcript and invalidate checkpoints.
          await invoke<UndoResult>('chat_undo', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
          });

          // 3. Reload messages from backend to sync UI with actual state.
          const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
          setCachedMessages(sessionMessagesRef.current, sessionId, msgs);
          syncVisible(sessionId);

          // 4. Clear edit state.
          setPendingEdit(null);

          // 5. Now send the new content as a fresh message.
          //    Add optimistic user message.
          const userMsg: Message = {
            id: `user-${Date.now()}`,
            role: 'user' as const,
            content: newContent,
            timestamp: new Date().toISOString(),
            tool_calls: [],
          };
          setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => [...prev, userMsg]);
          syncVisible(sessionId);

          const result = await invoke<ChatStarted>('chat_send', {
            message: newContent,
            sessionId,
            providerId: providerId ?? null,
          });
          // opStatus transitions to idle on chat:complete/chat:error via the bus handler.
          return result;
        } catch (e) {
          console.error('[chat] editAndResend failed:', e);
          setError(String(e));
          setOp('idle');
          // Rollback: reload from backend to restore consistent state.
          await loadMessages(sessionId);
          setPendingEdit(null);
          return null;
        }
      });
    },
    [pendingEdit, syncVisible, setOp, loadMessages],
  );

  // ------------------------------------------------------------------
  // undoToMessage -- transactional: undo to a specific message
  // ------------------------------------------------------------------

  const undoToMessage = useCallback(
    async (sessionId: string, messageId: string): Promise<UndoResult | null> => {
      console.log(`[chat] undoToMessage called, opStatus=${opStatusRef.current}, sessionId=${sessionId}, messageId=${messageId}`);
      if (opStatusRef.current !== 'idle') {
        console.warn(`[chat] undoToMessage blocked: opStatus=${opStatusRef.current}`);
        return null;
      }

      setOp('undoing');
      setError(null);

      return withSessionLock(sessionId, async () => {
        try {
          // 1. Find the checkpoint for the specific message.
          console.log('[chat] undoToMessage: finding checkpoint for message...');
          const checkpoint = await findCheckpointForMessage(sessionId, messageId, sessionMessagesRef.current);
          console.log('[chat] undoToMessage: checkpoint result', checkpoint);
          if (!checkpoint) {
            console.warn('[chat] No checkpoint found for message', messageId);
            setOp('idle');
            return null;
          }

          // 2. Backend undo to that checkpoint.
          console.log(`[chat] undoToMessage: calling chat_undo with checkpoint=${checkpoint.checkpoint_id}`);
          const result = await invoke<UndoResult>('chat_undo', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
          });
          console.log('[chat] undoToMessage: chat_undo result', result);

          // 3. Reload messages from backend.
          console.log('[chat] undoToMessage: reloading messages...');
          await loadMessages(sessionId);
          console.log('[chat] undoToMessage: loadMessages complete');
          setOp('idle');
          return result;
        } catch (e) {
          console.error('[chat] undoToMessage failed:', e);
          setError(String(e));
          setOp('idle');
          await loadMessages(sessionId);
          return null;
        }
      });
    },
    [loadMessages, setOp],
  );

  // ------------------------------------------------------------------
  // resendLastTurn -- transactional: keep user msg, re-run LLM
  // ------------------------------------------------------------------

  const resendLastTurn = useCallback(
    async (sessionId: string, messageId: string, content: string, providerId?: string): Promise<ChatStarted | null> => {
      console.log(`[chat] resendLastTurn called, opStatus=${opStatusRef.current}, sessionId=${sessionId}, messageId=${messageId}`);
      if (opStatusRef.current !== 'idle') {
        console.warn(`[chat] resendLastTurn blocked: opStatus=${opStatusRef.current}`);
        return null;
      }

      setOp('resending');
      setError(null);

      return withSessionLock(sessionId, async () => {
        try {
          // 0. Reload messages from backend to ensure cache has real IDs.
          const freshMsgs = await invoke<Message[]>('session_get_messages', { sessionId });
          setCachedMessages(sessionMessagesRef.current, sessionId, freshMsgs);

          // 1. Find checkpoint: use backend atomic command (primary),
          //    fall back to frontend multi-step lookup if the backend command fails.
          //    Resolve the messageId against freshMsgs to get a real backend UUID.
          let resolvedId = messageId;
          if (!freshMsgs.some((m) => m.id === messageId)) {
            const match = freshMsgs.find((m) => m.role === 'user' && m.content === content);
            if (match) {
              console.log(`[chat] resendLastTurn: resolved stale id=${messageId} -> ${match.id}`);
              resolvedId = match.id;
            }
          }

          let checkpoint: ChatCheckpointInfo | null = null;
          try {
            checkpoint = await invoke<ChatCheckpointInfo | null>(
              'chat_find_checkpoint_for_resend',
              { sessionId, userMessageContent: content, messageId: resolvedId },
            );
            console.log('[chat] resendLastTurn: backend checkpoint result', checkpoint);
          } catch (backendErr) {
            console.warn('[chat] resendLastTurn: backend checkpoint lookup failed, using frontend fallback:', backendErr);
            checkpoint = await findCheckpointForMessage(sessionId, resolvedId, sessionMessagesRef.current);
            console.log('[chat] resendLastTurn: frontend fallback checkpoint result', checkpoint);
          }

          if (!checkpoint) {
            console.warn('[chat] No checkpoint found for resend');
            setOp('idle');
            return null;
          }

          // 2. Remove the assistant reply from the cache.
          const keepCount = checkpoint.message_count_before + 1;
          const keptMsgs = freshMsgs.slice(0, keepCount);
          const removedMsgs = freshMsgs.slice(keepCount);
          console.log(`[chat] resendLastTurn: checkpoint.message_count_before=${checkpoint.message_count_before}, keepCount=${keepCount}, total=${freshMsgs.length}`);
          console.log('[chat] resendLastTurn: keeping:', keptMsgs.map(m => `[${m.role}] ${m.content.slice(0, 40)}...`));
          console.log('[chat] resendLastTurn: removing:', removedMsgs.map(m => `[${m.role}] ${m.content.slice(0, 40)}...`));
          setCachedMessages(sessionMessagesRef.current, sessionId, keptMsgs);
          syncVisible(sessionId);

          // 3. Backend resend: truncates transcript (keeps user msg), re-runs LLM.
          console.log(`[chat] resendLastTurn: calling chat_resend with checkpoint=${checkpoint.checkpoint_id}`);
          const result = await invoke<ChatStarted>('chat_resend', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
            providerId: providerId ?? null,
          });
          console.log('[chat] resendLastTurn: chat_resend result', result);
          // opStatus transitions to idle on chat:complete/chat:error via the bus handler.
          return result;
        } catch (e) {
          console.error('[chat] resendLastTurn failed:', e);
          setError(String(e));
          setOp('idle');
          // Reload to recover.
          await loadMessages(sessionId);
          return null;
        }
      });
    },
    [syncVisible, setOp, loadMessages],
  );

  // ------------------------------------------------------------------
  // restoreBranch -- transactional
  // ------------------------------------------------------------------

  const restoreBranch = useCallback(
    async (sessionId: string, checkpointId: string): Promise<RestoreResult | null> => {
      if (opStatusRef.current !== 'idle') {
        console.warn(`[chat] restoreBranch blocked: opStatus=${opStatusRef.current}`);
        return null;
      }

      setOp('restoring');
      setError(null);

      return withSessionLock(sessionId, async () => {
        try {
          const result = await invoke<RestoreResult>('chat_restore_branch', {
            sessionId,
            checkpointId,
          });
          await loadMessages(sessionId);
          setOp('idle');
          return result;
        } catch (e) {
          console.error('[chat] restoreBranch failed:', e);
          setError(String(e));
          setOp('idle');
          return null;
        }
      });
    },
    [loadMessages, setOp],
  );

  return {
    messages: visibleMessages,
    isStreaming,
    isLoadingMessages,
    streamingSessionIds,
    activeRunId,
    error,
    opStatus,
    pendingEdit,
    sendMessage,
    cancelRun,
    loadMessages,
    clearMessages,
    editMessage,
    cancelEdit,
    editAndResend,
    undoToMessage,
    resendLastTurn,
    restoreBranch,
  };
}
