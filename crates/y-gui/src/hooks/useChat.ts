// Custom hook for chat functionality -- per-session streaming state.
//
// Architecture (post-refactoring):
// - Module-level ChatBus singleton handles Tauri event listeners (chatBus.ts).
// - Per-session message cache and lock utilities (chatHelpers.ts).
// - Operation state machine prevents illegal concurrent operations.
// - All compound operations are transactional: backend-first, then UI.

import { useState, useCallback, useEffect, useRef, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type {
  Message,
  ChatStarted,
  UndoResult,
  RestoreResult,
  ThinkingEffort,
  PlanMode,
  Attachment,
} from '../types';
import {
  chatBusState,
  chatBusSubscribers,
  processedCancelledRuns,
  type ChatBusSubscriber,
} from './chatBus';
import { hasPendingRunForSession } from './chatRunState';
import {
  getCachedMessages,
  setCachedMessages,
  mergeSkillsFromCache,
  withSessionLock,
  findCheckpointForMessage,
} from './chatHelpers';
import {
  shouldDisplayStreamingAgent,
  type ToolResultRecord,
} from './chatStreamTypes';
import { upsertToolResultRecord, upsertToolResultSegment } from './toolResultUpdates';
import type { InterleavedSegment } from './useInterleavedSegments';

// Re-export ChatBusEvent for consumers that need the union type.
export type { ChatBusEvent } from './chatBus';

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
  | 'restoring'
  | 'compacting';

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
  /** Tool results from the current streaming run (for inline cards). */
  toolResults: ToolResultRecord[];
  /** Get event-ordered segments for the active streaming session.
   *  Returns null if no tool calls are present. Built from event arrival
   *  order (stream_delta -> text, tool_result -> tool card) so the
   *  interleaving is inherently correct without character offsets. */
  getStreamSegments: () => InterleavedSegment[] | null;
  sendMessage: (message: string, sessionId: string, providerId?: string, skills?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode) => Promise<ChatStarted | null>;
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
  /** All context reset points for the active session (empty = no resets). */
  contextResetPoints: number[];
  /** Add a new context reset point at the current message position. */
  addContextReset: () => void;
  /** Compaction display items (divider + summary) injected after compaction completes. */
  compactPoints: CompactInfo[];
  /** Add a compaction point at the current message position (called by handleCommand). */
  addCompactPoint: (info: Omit<CompactInfo, 'atIndex'>) => void;
  /** Set the operation status (used by handlers for compacting state). */
  setOp: (status: ChatOpStatus) => void;
}

/** Info about a completed compaction for rendering a divider + summary bubble. */
export interface CompactInfo {
  /** Index in the message list where the divider should appear. */
  atIndex: number;
  messagesPruned: number;
  messagesCompacted: number;
  tokensSaved: number;
  summary: string;
}

function toToolResultMetadata(records: ToolResultRecord[]): Array<Record<string, unknown>> {
  return records.map((tr) => {
    const entry: Record<string, unknown> = {
      name: tr.name,
      arguments: tr.arguments ?? '',
      success: tr.success,
      duration_ms: tr.durationMs,
      result_preview: tr.resultPreview,
    };
    if (tr.urlMeta) {
      try {
        entry.url_meta = JSON.parse(tr.urlMeta) as Record<string, unknown>;
      } catch {
        entry.url_meta = tr.urlMeta;
      }
    }
    if (tr.metadata) {
      entry.metadata = tr.metadata;
    }
    return entry;
  });
}

function mergeToolResultMetadata(
  backend: unknown,
  streamed: ToolResultRecord[] | undefined,
): Array<Record<string, unknown>> | undefined {
  const backendRecords = Array.isArray(backend)
    ? backend.filter(
      (entry): entry is Record<string, unknown> =>
        entry != null && typeof entry === 'object',
    )
    : [];
  const streamRecords = streamed ? toToolResultMetadata(streamed) : [];

  if (backendRecords.length === 0) {
    return streamRecords.length > 0 ? streamRecords : undefined;
  }
  if (streamRecords.length === 0) {
    return backendRecords;
  }

  const merged: Array<Record<string, unknown>> = [];
  const seen = new Set<string>();

  for (const entry of [...streamRecords, ...backendRecords]) {
    const metadata = entry.metadata;
    const metadataKey = metadata == null ? '' : JSON.stringify(metadata);
    const key = [
      String(entry.name ?? ''),
      String(entry.arguments ?? ''),
      String(entry.result_preview ?? ''),
      metadataKey,
    ].join('::');
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(entry);
  }

  return merged;
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

  // Operation state machine -- per-session.
  // A ref Map tracks each session's opStatus so that switching sessions
  // restores the correct state instead of leaking one session's status
  // into another (e.g. Session A is streaming, user switches to idle Session B).
  const opStatusMapRef = useRef(new Map<string, ChatOpStatus>());
  const [opStatus, setOpStatus] = useState<ChatOpStatus>('idle');
  const opStatusRef = useRef<ChatOpStatus>('idle');
  const setOp = useCallback((status: ChatOpStatus) => {
    opStatusRef.current = status;
    setOpStatus(status);
    // Persist into the per-session map.
    const sid = activeSessionIdRef.current;
    if (sid) {
      opStatusMapRef.current.set(sid, status);
    }
  }, []);

  /** Update opStatus for a specific session. Only touches visible state
   *  if that session is currently active; otherwise just updates the map. */
  const setOpForSession = useCallback((sessionId: string, status: ChatOpStatus) => {
    opStatusMapRef.current.set(sessionId, status);
    if (sessionId === activeSessionIdRef.current) {
      opStatusRef.current = status;
      setOpStatus(status);
    }
  }, []);

  // Pending edit state (exposed to InputArea for banner).
  const [pendingEdit, setPendingEdit] = useState<PendingEdit | null>(null);

  // Per-session tool results from progress events (for inline tool call cards).
  const toolResultsRef = useRef(new Map<string, ToolResultRecord[]>());
  const [visibleToolResults, setVisibleToolResults] = useState<ToolResultRecord[]>([]);

  // Per-session event-ordered segments (text + tool_result interleaved by
  // arrival order). Mutated in place on every stream_delta and tool_result
  // event. Read via getStreamSegments() during render.
  const streamSegsRef = useRef(new Map<string, InterleavedSegment[]>());
  // Counter bumped on structural changes (tool_result added) to force
  // re-renders in consuming components.
  const [streamSegsVersion, setStreamSegsVersion] = useState(0);

  const getStreamSegments = useCallback((): InterleavedSegment[] | null => {
    const sid = activeSessionIdRef.current;
    if (!sid) return null;
    const segs = streamSegsRef.current.get(sid);
    if (!segs || segs.length === 0) return null;
    // Only return segments when there are tool results or reasoning
    // (otherwise plain text handled by the bubble directly).
    if (segs.some((s) => s.type === 'tool_result' || s.type === 'reasoning')) return segs;
    return null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamSegsVersion]);

  // Per-session context reset points (list of message indices where context was reset).
  const contextResetMapRef = useRef(new Map<string, number[]>());
  const [contextResetPoints, setContextResetPoints] = useState<number[]>([]);

  // Per-session compaction points (divider + summary info after /compact completes).
  const compactMapRef = useRef(new Map<string, CompactInfo[]>());
  const [compactPoints, setCompactPoints] = useState<CompactInfo[]>([]);

  // Keep a ref in sync with activeSessionId.
  const activeSessionIdRef = useRef<string | null>(activeSessionId);
  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
    // Cancel edit mode when switching sessions.
    if (pendingEdit) {
      setPendingEdit(null);
    }
    // Restore the new session's opStatus (default: idle).
    // This prevents a running session's status from leaking into
    // an idle session's InputArea, which would leave it disabled.
    const restoredOp = activeSessionId
      ? (opStatusMapRef.current.get(activeSessionId) ?? 'idle')
      : 'idle';
    opStatusRef.current = restoredOp;
    setOpStatus(restoredOp);
    // Restore tool results for the new session.
    if (activeSessionId) {
      setVisibleToolResults(toolResultsRef.current.get(activeSessionId) ?? []);
    } else {
      setVisibleToolResults([]);
    }
    // Clear error from the previous session so it does not leak
    // into the newly selected session's chat panel.
    setError(null);
    // Restore context reset points for the new session.
    // First check the in-memory map, then asynchronously load from backend.
    if (activeSessionId) {
      const cached = contextResetMapRef.current.get(activeSessionId);
      if (cached) {
        setContextResetPoints(cached);
      } else {
        // Load from backend (persisted across restarts).
        invoke<number | null>('session_get_context_reset', { sessionId: activeSessionId })
          .then((idx) => {
            if (activeSessionIdRef.current !== activeSessionId) return;
            if (idx != null) {
              const points = [idx];
              contextResetMapRef.current.set(activeSessionId, points);
              setContextResetPoints(points);
            } else {
              setContextResetPoints([]);
            }
          })
          .catch((e) => console.warn('[chat] failed to load context reset:', e));
      }
    } else {
      setContextResetPoints([]);
    }
    // Restore compact points for the new session.
    if (activeSessionId) {
      setCompactPoints(compactMapRef.current.get(activeSessionId) ?? []);
    } else {
      setCompactPoints([]);
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
        // Clear tool results and stream segments for the new run.
        toolResultsRef.current.set(event.session_id, []);
        streamSegsRef.current.set(event.session_id, []);
        if (event.session_id === activeSessionIdRef.current) {
          setVisibleToolResults([]);
        }
        console.log('[chat] run started, run_id =', event.run_id, 'session =', event.session_id);
      } else if (event.type === 'stream_delta') {
        if (!shouldDisplayStreamingAgent(event.agent_name)) {
          return;
        }
        const sid = event.session_id;
        // Append to event-ordered segments (text segment).
        const segs = streamSegsRef.current.get(sid);
        if (segs) {
          // When content arrives, mark any in-progress reasoning segment as done.
          const lastSeg = segs[segs.length - 1];
          if (lastSeg && lastSeg.type === 'reasoning' && lastSeg.isStreaming) {
            lastSeg.isStreaming = false;
            if (lastSeg._startTs) {
              lastSeg.durationMs = Date.now() - lastSeg._startTs;
            }
          }
          // Append text.
          if (lastSeg && lastSeg.type === 'text') {
            lastSeg.text += event.content;
          } else {
            segs.push({ type: 'text', text: event.content });
          }
        }
        setCachedMessages(sessionMessagesRef.current, sid, (prev) => {
          const streamingId = `streaming-${sid}`;
          const lastIdx = prev.findIndex((m) => m.id === streamingId);
          if (lastIdx >= 0) {
            const updated = [...prev];
            const existing = updated[lastIdx];
            // When first content delta arrives, mark reasoning as done.
            const meta = { ...(existing.metadata || {}) };
            if (meta._reasoningStartTs && !meta._reasoningDoneTs) {
              meta._reasoningDoneTs = Date.now();
              meta._reasoningDurationMs = (meta._reasoningDoneTs as number) - (meta._reasoningStartTs as number);
            }
            updated[lastIdx] = {
              ...existing,
              content: existing.content + event.content,
              metadata: meta,
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
      } else if (event.type === 'stream_reasoning_delta') {
        if (!shouldDisplayStreamingAgent(event.agent_name)) {
          return;
        }
        const sid = event.session_id;
        console.log(`[chat] stream_reasoning_delta: session=${sid}, len=${event.content.length}`);
        // Push/extend a reasoning segment in event-ordered segments.
        const segs = streamSegsRef.current.get(sid);
        if (segs) {
          const lastSeg = segs[segs.length - 1];
          if (lastSeg && lastSeg.type === 'reasoning' && lastSeg.isStreaming) {
            // Extend existing in-progress reasoning segment.
            lastSeg.content += event.content;
          } else {
            // New reasoning segment (new iteration's reasoning).
            segs.push({
              type: 'reasoning',
              content: event.content,
              isStreaming: true,
              _startTs: Date.now(),
            });
            setStreamSegsVersion((v) => v + 1);
          }
        }
        // Also merge into metadata for backward compat (copy, etc.).
        setCachedMessages(sessionMessagesRef.current, sid, (prev) => {
          const streamingId = `streaming-${sid}`;
          const lastIdx = prev.findIndex((m) => m.id === streamingId);
          if (lastIdx >= 0) {
            const updated = [...prev];
            const existing = updated[lastIdx];
            const meta = { ...(existing.metadata || {}) };
            meta.reasoning_content = ((meta.reasoning_content as string) || '') + event.content;
            if (!meta._reasoningStartTs) {
              meta._reasoningStartTs = Date.now();
            }
            updated[lastIdx] = { ...existing, metadata: meta };
            return updated;
          }
          return [
            ...prev,
            {
              id: streamingId,
              role: 'assistant' as const,
              content: '',
              timestamp: new Date().toISOString(),
              tool_calls: [],
              _streaming: true,
              metadata: {
                reasoning_content: event.content,
                _reasoningStartTs: Date.now(),
              },
            } as Message,
          ];
        });
        syncVisible(sid);
      } else if (event.type === 'complete') {
        const payload = event.payload;
        // Resolve session: prefer payload (always available from backend),
        // fall back to bus mapping for older event formats.
        const sessionId = payload.session_id || chatBusState.runToSession[payload.run_id] || undefined;
        const sessionStillActive = sessionId
          ? hasPendingRunForSession(chatBusState, sessionId)
          : false;
        console.log(`[chat] complete: run_id=${payload.run_id}, session=${sessionId}, opStatus=${opStatusRef.current}`);

        if (sessionStillActive) {
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          return;
        }

        if (sessionId) {
          // Merge streaming content with backend messages.
          // The streaming-{sid} message accumulates text from ALL LLM
          // iterations in the tool-call loop. The backend only persists
          // the final assistant message. We merge: use backend messages
          // as the authoritative source for IDs/metadata, but preserve
          // the full accumulated streaming content so multi-iteration
          // text (e.g. "I'll check" + analysis) is not lost.
          (async () => {
            try {
              const msgs = await invoke<Message[]>('session_get_messages', { sessionId });

              // Preserve skill tags from optimistic user messages.
              const mergedMsgs = mergeSkillsFromCache(msgs, sessionMessagesRef.current, sessionId);

              // Grab the accumulated streaming content before overwriting.
              const streamingId = `streaming-${sessionId}`;
              const cachedMessages = getCachedMessages(sessionMessagesRef.current, sessionId);
              const streamingMsg = cachedMessages.find((m) => m.id === streamingId);
              const accumulatedContent = streamingMsg?.content ?? '';
              const snapToolResults = toolResultsRef.current.get(sessionId);

              // If there was accumulated streaming content and the backend
              // has a final assistant message, check if the streaming content
              // carries extra text from prior tool-call iterations.
              if (mergedMsgs.length > 0) {
                const lastMsg = mergedMsgs[mergedMsgs.length - 1];
                if (lastMsg.role === 'assistant') {
                  // Merge streaming reasoning metadata (timing info is
                  // client-only and not persisted by the backend).
                  const streamMeta = streamingMsg?.metadata;
                  const mergedMeta = { ...(lastMsg.metadata || {}) };
                  if (streamMeta) {
                    if (streamMeta._reasoningDurationMs) {
                      mergedMeta._reasoningDurationMs = streamMeta._reasoningDurationMs;
                    }
                    if (streamMeta._reasoningDoneTs) {
                      mergedMeta._reasoningDoneTs = streamMeta._reasoningDoneTs;
                    }
                    // Fallback: if backend lacks reasoning_content but streaming had it
                    if (!mergedMeta.reasoning_content && streamMeta.reasoning_content) {
                      mergedMeta.reasoning_content = streamMeta.reasoning_content;
                    }
                  }

                  const mergedToolResults = mergeToolResultMetadata(
                    mergedMeta.tool_results,
                    snapToolResults,
                  );
                  if (mergedToolResults) {
                    mergedMeta.tool_results = mergedToolResults;
                  }

                  if (accumulatedContent.length > lastMsg.content.length) {
                    // The streaming content has more text (from earlier
                    // iterations). Use it as the display content but keep
                    // the backend message's metadata (enriched with timing).
                    mergedMsgs[mergedMsgs.length - 1] = {
                      ...lastMsg,
                      content: accumulatedContent,
                      metadata: mergedMeta,
                    };
                  } else if (Object.keys(mergedMeta).length > Object.keys(lastMsg.metadata || {}).length) {
                    // Content is same but we have extra metadata to merge.
                    mergedMsgs[mergedMsgs.length - 1] = {
                      ...lastMsg,
                      metadata: mergedMeta,
                    };
                  }
                }
              }

              setCachedMessages(sessionMessagesRef.current, sessionId, mergedMsgs);
              if (activeSessionIdRef.current === sessionId) {
                startTransition(() => {
                  setVisibleMessages(mergedMsgs);
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
                  tool_calls: payload.tool_calls.map((tc: { name: string }) => ({
                    id: tc.name,
                    name: tc.name,
                    arguments: '',
                  })),
                  model: payload.model,
                  provider_id: payload.provider_id,
                  tokens: { input: payload.input_tokens, output: payload.output_tokens },
                  cost: payload.cost_usd,
                  context_window: payload.context_window,
                  metadata: {
                    tool_results: mergeToolResultMetadata(
                      [],
                      toolResultsRef.current.get(sessionId),
                    ) ?? [],
                  },
                };
                if (filtered.some((m) => m.id === newMsg.id)) return filtered;
                return [...filtered, newMsg];
              });
              syncVisible(sessionId);
            } finally {
              // Transition to idle AFTER the cache is updated, not before.
              // Use session-aware setter so a background session's completion
              // does not reset the active session's input state.
              if (!sessionStillActive) {
                setOpForSession(sessionId, 'idle');
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
        // Resolve session: prefer payload, fall back to bus mapping.
        // For cancels the backend sends empty string, so the fallback is needed.
        const sessionId = payload.session_id || chatBusState.runToSession[payload.run_id] || undefined;
        const sessionStillActive = sessionId
          ? hasPendingRunForSession(chatBusState, sessionId)
          : false;
        const isCancelled = payload.error === 'Cancelled';

        if (sessionStillActive) {
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          return;
        }

        // Deduplicate cancel events: `chat_cancel` emits one immediately,
        // and the spawned LLM task emits another when it detects the
        // cancellation token. Skip the second one entirely.
        if (isCancelled && processedCancelledRuns.has(payload.run_id)) {
          // Already handled -- just ensure streaming state is cleared.
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          if (activeRunIdRef.current === payload.run_id) {
            activeRunIdRef.current = null;
            setActiveRunId(null);
          }
          return;
        }
        if (isCancelled) {
          processedCancelledRuns.add(payload.run_id);
          // Clean up after a delay so we don't leak memory.
          setTimeout(() => processedCancelledRuns.delete(payload.run_id), 30_000);
        }

        if (sessionId) {
          if (isCancelled) {
            // Stop/cancel: preserve any streamed content by finalizing the
            // streaming message instead of deleting it. This keeps the
            // partially-streamed text visible and treats the run as complete.
            // Snapshot tool results so they survive in the finalized message
            // metadata even after visibleToolResults is cleared by a new run.
            const snapToolResults = toolResultsRef.current.get(sessionId);

            setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => {
              const streamingId = `streaming-${sessionId}`;
              // reasoning content is merged into the streaming message's metadata.
              return prev.map((m) => {
                if (m.id === streamingId && m.content) {
                  const meta = { ...(m.metadata ?? {}) };
                  const mergedToolResults = mergeToolResultMetadata(
                    meta.tool_results,
                    snapToolResults,
                  );
                  if (mergedToolResults) {
                    meta.tool_results = mergedToolResults;
                  }
                  return {
                    ...m,
                    id: `cancelled-${payload.run_id}`,
                    metadata: meta,
                    _streaming: undefined,
                  } as Message;
                }
                if (m.id === streamingId) return null;
                return m;
              }).filter(Boolean) as Message[];
            });

            // Reload from backend so the cache has real backend IDs
            // (the cancelled assistant message only exists in the cache,
            // the backend has the user message with its real UUID).
            // This ensures subsequent resend/undo can find the message.
            (async () => {
              try {
                const backendMsgs = await invoke<Message[]>('session_get_messages', { sessionId });
                // Merge: keep backend messages + any cancelled assistant msg.
                const cancelledMsg = getCachedMessages(sessionMessagesRef.current, sessionId)
                  .find((m) => m.id === `cancelled-${payload.run_id}`);
                const merged = cancelledMsg ? [...backendMsgs, cancelledMsg] : backendMsgs;
                setCachedMessages(sessionMessagesRef.current, sessionId, merged);
                if (activeSessionIdRef.current === sessionId) {
                  startTransition(() => setVisibleMessages(merged));
                }
              } catch (e) {
                console.error('[chat] cancel: failed to reload messages:', e);
              }
            })();
          } else {
            // Non-cancel error: preserve any streamed content by
            // finalizing the streaming message instead of deleting it.
            // This keeps reasoning, tool call cards, and partial text
            // visible so the user can see what happened before the error.
            const snapToolResultsErr = toolResultsRef.current.get(sessionId);

            setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => {
              const streamingId = `streaming-${sessionId}`;
              return prev.map((m) => {
                if (m.id === streamingId && m.content) {
                  const meta = { ...(m.metadata ?? {}) };
                  const mergedToolResults = mergeToolResultMetadata(
                    meta.tool_results,
                    snapToolResultsErr,
                  );
                  if (mergedToolResults) {
                    meta.tool_results = mergedToolResults;
                  }
                  return {
                    ...m,
                    id: `error-${payload.run_id || Date.now()}`,
                    metadata: meta,
                    _streaming: undefined,
                  } as Message;
                }
                if (m.id === streamingId) return null;
                return m;
              }).filter(Boolean) as Message[];
            });
            syncVisible(sessionId);

            // Async reload: merge backend messages with the preserved
            // error message so both are visible.
            (async () => {
              try {
                const backendMsgs = await invoke<Message[]>('session_get_messages', { sessionId });
                const errorMsg = getCachedMessages(sessionMessagesRef.current, sessionId)
                  .find((m) => m.id.startsWith('error-'));
                const merged = mergeSkillsFromCache(backendMsgs, sessionMessagesRef.current, sessionId);
                const final_ = errorMsg ? [...merged, errorMsg] : merged;
                setCachedMessages(sessionMessagesRef.current, sessionId, final_);
                if (activeSessionIdRef.current === sessionId) {
                  startTransition(() => setVisibleMessages(final_));
                }
              } catch (reloadErr) {
                console.error('[chat] error handler: failed to reload messages:', reloadErr);
              }
            })();
          }
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
        if (sessionId) {
          if (!sessionStillActive) {
            setOpForSession(sessionId, 'idle');
          }
        } else if (opStatusRef.current !== 'idle') {
          setOp('idle');
        }
      } else if (event.type === 'tool_result') {
        // Accumulate tool results for inline card rendering.
        const sid = event.session_id;
        const record: ToolResultRecord = {
          name: event.name,
          arguments: event.input_preview,
          success: event.success,
          durationMs: event.duration_ms,
          resultPreview: event.result_preview,
          urlMeta: event.url_meta,
          metadata: event.metadata,
        };
        const existing = toolResultsRef.current.get(sid) ?? [];
        const nextToolResults = upsertToolResultRecord(existing, record);
        toolResultsRef.current.set(sid, nextToolResults.records);
        if (sid === activeSessionIdRef.current) {
          setVisibleToolResults(nextToolResults.records);
        }
        // Push or replace a tool_result segment and bump version to force re-render.
        const segs = streamSegsRef.current.get(sid);
        if (segs) {
          let preparedSegs = segs;
          // When a tool result arrives, mark any in-progress reasoning segment
          // as done. This handles the case where the LLM thinks and then
          // directly issues tool calls without emitting any text content
          // (stream_delta) in between -- without this, the ThinkingCard would
          // stay in "Thinking..." state indefinitely.
          const lastSeg = segs[segs.length - 1];
          if (lastSeg && lastSeg.type === 'reasoning' && lastSeg.isStreaming) {
            preparedSegs = [...segs];
            preparedSegs[preparedSegs.length - 1] = {
              ...lastSeg,
              isStreaming: false,
              durationMs: lastSeg._startTs ? Date.now() - lastSeg._startTs : lastSeg.durationMs,
            };
          }
          const nextSegments = upsertToolResultSegment(preparedSegs, record);
          streamSegsRef.current.set(sid, nextSegments.segments);
          setStreamSegsVersion((v) => v + 1);
        }
      }
    };

    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  }, [syncVisible, setOp, setOpForSession]);

  // ------------------------------------------------------------------
  // Safety timeout: if opStatus stays non-idle for too long without any
  // bus activity, force-reset to idle so the session is never permanently
  // stuck. This guards against edge cases where the backend fails to emit
  // a terminal event (e.g. task panic, IPC failure).
  // ------------------------------------------------------------------

  useEffect(() => {
    if (opStatus === 'idle') return;

    const STUCK_TIMEOUT_MS = 5 * 60 * 1000; // 5 minutes
    const timer = setTimeout(() => {
      if (opStatusRef.current !== 'idle') {
        console.warn(
          `[chat] safety timeout: opStatus=${opStatusRef.current} stuck for ${STUCK_TIMEOUT_MS}ms, forcing idle`,
        );
        setOp('idle');
        // Also clear streaming state for the active session so the UI
        // no longer shows the streaming indicator.
        const sid = activeSessionIdRef.current;
        if (sid) {
          chatBusState.streamingSessions.delete(sid);
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        }
        activeRunIdRef.current = null;
        setActiveRunId(null);
      }
    }, STUCK_TIMEOUT_MS);

    return () => clearTimeout(timer);
  }, [opStatus, setOp]);

  // ------------------------------------------------------------------
  // Core operations
  // ------------------------------------------------------------------

  const loadMessages = useCallback(async (sessionId: string) => {
    activeSessionIdRef.current = sessionId;

    const cachedMsgs = getCachedMessages(sessionMessagesRef.current, sessionId);
    // Show the loading skeleton only when the cache is empty -- if we already
    // have cached (optimistic) messages we want them to stay visible rather
    // than being replaced by a skeleton flash.
    const showSkeleton = cachedMsgs.length === 0;

    startTransition(() => {
      setVisibleMessages(cachedMsgs);
      if (showSkeleton) {
        setIsLoadingMessages(true);
      }
    });

    try {
      // Fetch messages and persisted context reset index in parallel.
      const [msgs, resetIdx] = await Promise.all([
        invoke<Message[]>('session_get_messages', { sessionId }),
        invoke<number | null>('session_get_context_reset', { sessionId }),
      ]);

      // Restore persisted context reset points.
      if (resetIdx != null) {
        const points = [resetIdx];
        contextResetMapRef.current.set(sessionId, points);
        if (activeSessionIdRef.current === sessionId) {
          setContextResetPoints(points);
        }
      } else if (!contextResetMapRef.current.has(sessionId)) {
        // No persisted reset and no in-memory entry: ensure clean state.
        contextResetMapRef.current.set(sessionId, []);
        if (activeSessionIdRef.current === sessionId) {
          setContextResetPoints([]);
        }
      }

      // Preserve skill tags from cached messages.
      const mergedMsgs = mergeSkillsFromCache(msgs, sessionMessagesRef.current, sessionId);
      console.log(`[chat] loadMessages: got ${mergedMsgs.length} messages for session=${sessionId}, active=${activeSessionIdRef.current}`);
      if (activeSessionIdRef.current === sessionId) {
        const streamingId = `streaming-${sessionId}`;
        // Re-read from cache (may have been updated by sendMessage in the meantime).
        const currentCached = getCachedMessages(sessionMessagesRef.current, sessionId);
        const existingStreaming = currentCached.find((m) => m.id === streamingId);

        // Preserve optimistic user messages (id starts with "user-") that
        // exist in the cache but are not yet in the backend response.
        // This happens when sendMessage adds an optimistic message and
        // loadMessages races with the backend persistence (common for new
        // sessions where the first message hasn't been saved yet).
        const backendIds = new Set(mergedMsgs.map((m) => m.id));
        // Content-based dedup: if the backend already persisted a user
        // message with the same content, drop the optimistic copy.
        const backendUserContents = new Set(
          mergedMsgs.filter((m) => m.role === 'user').map((m) => m.content),
        );
        const optimisticUserMsgs = currentCached.filter(
          (m) =>
            m.id.startsWith('user-') &&
            !backendIds.has(m.id) &&
            !backendUserContents.has(m.content),
        );

        let merged = [...mergedMsgs, ...optimisticUserMsgs];
        if (existingStreaming) {
          merged = [...merged, existingStreaming];
        }

        setCachedMessages(sessionMessagesRef.current, sessionId, merged);
        startTransition(() => {
          setVisibleMessages(merged);
        });
      } else {
        console.log(`[chat] loadMessages: session mismatch, skipping visible update (active=${activeSessionIdRef.current}, requested=${sessionId})`);
        setCachedMessages(sessionMessagesRef.current, sessionId, mergedMsgs);
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

  // ------------------------------------------------------------------
  // invalidateStaleContextResets -- remove reset points at or after a
  // given message count. Called by undo / edit / resend operations so
  // that context reset markers do not survive when the user goes back
  // to a point before the reset was placed.
  // ------------------------------------------------------------------

  const invalidateStaleContextResets = useCallback((sessionId: string, newMsgCount: number) => {
    const existing = contextResetMapRef.current.get(sessionId) ?? [];
    const kept = existing.filter((idx) => idx < newMsgCount);
    if (kept.length === existing.length) return; // nothing changed
    contextResetMapRef.current.set(sessionId, kept);
    if (activeSessionIdRef.current === sessionId) {
      setContextResetPoints(kept);
    }
    // Persist: store the latest kept point (or clear if none remain).
    const persistIdx = kept.length > 0 ? kept[kept.length - 1] : null;
    invoke('session_set_context_reset', { sessionId, index: persistIdx })
      .catch((e) => console.error('[chat] failed to clear stale context reset:', e));
  }, []);

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

  // Synchronous guard for sendMessage -- prevents concurrent sends even when
  // React state (opStatusRef) has not yet flushed.
  const sendingRef = useRef(false);

  const sendMessage = useCallback(
    async (message: string, sessionId: string, providerId?: string, skills?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode): Promise<ChatStarted | null> => {
      // Guard: block if any operation is already in progress, including a
      // prior send.  The previous guard also allowed 'sending' through,
      // which could cause duplicate LLM calls when rapid double-fires
      // occurred (e.g. IME Enter key events).
      if (opStatusRef.current !== 'idle') {
        console.warn(`[chat] sendMessage blocked: opStatus=${opStatusRef.current}`);
        return null;
      }
      // Synchronous flag: opStatusRef is updated via React setState which
      // is asynchronous.  Two sendMessage calls in the same microtask
      // would both see opStatusRef.current === 'idle'.  This ref provides
      // an immediate guard.
      if (sendingRef.current) {
        console.warn('[chat] sendMessage blocked: already sending (ref guard)');
        return null;
      }
      sendingRef.current = true;

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
        skills: skills && skills.length > 0 ? skills : undefined,
        metadata: attachments && attachments.length > 0 ? { attachments } : undefined,
      };
      setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => [...prev, userMsg]);
      syncVisible(sessionId);

      try {
        // Pass context reset index so the backend trims history for fresh context.
        // Use the latest (most recent) reset point.
        const resetPoints = contextResetMapRef.current.get(sessionId) ?? [];
        const resetIdx = resetPoints.length > 0 ? resetPoints[resetPoints.length - 1] : null;
        console.log('[chat] sendMessage: planMode =', planMode, '-> sending:', planMode ?? null);
        const result = await invoke<ChatStarted>('chat_send', {
          message,
          sessionId,
          providerId: providerId ?? null,
          skills: skills && skills.length > 0 ? skills : null,
          knowledgeCollections: knowledgeCollections && knowledgeCollections.length > 0 ? knowledgeCollections : null,
          contextStartIndex: resetIdx,
          thinkingEffort: thinkingEffort ?? null,
          attachments: attachments && attachments.length > 0 ? attachments : null,
          planMode: planMode ?? null,
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
      } finally {
        sendingRef.current = false;
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
            // No checkpoint: this happens after a cancelled run where the
            // assistant message was never persisted and no checkpoint was
            // created. The backend transcript ends with the user message.
            // Truncate to remove the user message and re-send with new content.
            console.warn('[chat] No checkpoint found for edit -- using truncate fallback');
            const freshMsgs = await invoke<Message[]>('session_get_messages', { sessionId });
            let userIdx = freshMsgs.findIndex((m) => m.id === edit.messageId);
            if (userIdx < 0) {
              // Optimistic IDs won't match backend UUIDs; match by content+role.
              const cachedMsg = getCachedMessages(sessionMessagesRef.current, sessionId)
                .find((m) => m.id === edit.messageId);
              if (cachedMsg) {
                userIdx = freshMsgs.findIndex(
                  (m) => m.role === cachedMsg.role && m.content === cachedMsg.content,
                );
              }
            }

            if (userIdx >= 0) {
              await invoke('session_truncate_messages', {
                sessionId,
                keepCount: userIdx,
              });
              const keptMsgs = freshMsgs.slice(0, userIdx);
              setCachedMessages(sessionMessagesRef.current, sessionId, keptMsgs);
              invalidateStaleContextResets(sessionId, keptMsgs.length);
              syncVisible(sessionId);
              setPendingEdit(null);

              // Add optimistic user message with new content.
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
              return result;
            }

            // Could not find the user message at all -- abort gracefully.
            console.warn('[chat] editAndResend: fallback failed, user message not found');
            setPendingEdit(null);
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
          invalidateStaleContextResets(sessionId, msgs.length);
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
    [pendingEdit, syncVisible, setOp, loadMessages, invalidateStaleContextResets],
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
            // No checkpoint: this can happen after a cancelled run where
            // the assistant message was never persisted. Fall back to
            // direct transcript truncation.
            console.warn('[chat] No checkpoint found for message', messageId, '-- trying direct truncation fallback');
            const backendMsgs = await invoke<Message[]>('session_get_messages', { sessionId });
            let targetIdx = backendMsgs.findIndex((m) => m.id === messageId);
            if (targetIdx < 0) {
              // Content-based fallback: match by role + content.
              const cached = sessionMessagesRef.current.get(sessionId)?.find((cm) => cm.id === messageId);
              if (cached) {
                targetIdx = backendMsgs.findIndex((m) => m.role === cached.role && m.content === cached.content);
              }
            }
            if (targetIdx >= 0) {
              // Truncate to before this user message.
              await invoke('session_truncate_messages', { sessionId, keepCount: targetIdx });
              invalidateStaleContextResets(sessionId, targetIdx);
              await loadMessages(sessionId);
              setOp('idle');
              return { messages_removed: targetIdx, restored_turn_number: 0, files_restored: 0 } as UndoResult;
            }
            console.warn('[chat] undoToMessage: fallback truncation also failed');
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
          // Invalidate context resets that fall at or after the restored point.
          invalidateStaleContextResets(sessionId, checkpoint.message_count_before);
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
    [loadMessages, setOp, invalidateStaleContextResets],
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

          // 1. Find checkpoint via the atomic backend command.
          //    findCheckpointForMessage now delegates to chat_find_checkpoint_for_resend
          //    which handles ID resolution and content-based fallback internally.
          const checkpoint = await findCheckpointForMessage(sessionId, messageId, sessionMessagesRef.current);
          console.log('[chat] resendLastTurn: checkpoint result', checkpoint);

          if (!checkpoint) {
            // No checkpoint: this happens after a cancelled run where the
            // assistant message was never persisted and no checkpoint was
            // created. The backend transcript ends with the user message.
            // We can simply truncate to remove that user message and
            // re-send via chat_send.
            console.warn('[chat] No checkpoint found for resend -- using direct re-send fallback');
            let userIdx = freshMsgs.findIndex((m) => m.id === messageId);
            if (userIdx < 0) {
              userIdx = freshMsgs.findIndex((m) => m.role === 'user' && m.content === content);
            }
            if (userIdx >= 0) {
              // Truncate to remove the user message.
              await invoke('session_truncate_messages', {
                sessionId,
                keepCount: userIdx,
              });
              // Update cache to remove the user message and any cancelled assistant msg.
              const keptMsgs = freshMsgs.slice(0, userIdx);
              setCachedMessages(sessionMessagesRef.current, sessionId, keptMsgs);
              invalidateStaleContextResets(sessionId, keptMsgs.length);
              syncVisible(sessionId);

              // Add optimistic user message so the bubble stays visible while
              // the backend processes the re-send (chat_send persists a new
              // user message but sync only happens on chat:complete).
              const userMsg: Message = {
                id: `user-${Date.now()}`,
                role: 'user' as const,
                content,
                timestamp: new Date().toISOString(),
                tool_calls: [],
              };
              setCachedMessages(sessionMessagesRef.current, sessionId, (prev) => [...prev, userMsg]);
              syncVisible(sessionId);

              // opStatus stays 'resending' -- the bus handler will set idle on complete/error.
              const result = await invoke<ChatStarted>('chat_send', {
                message: content,
                sessionId,
                providerId: providerId ?? null,
              });
              console.log('[chat] resendLastTurn: fallback chat_send result', result);
              return result;
            }
            console.warn('[chat] resendLastTurn: fallback also failed, user message not found');
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
          invalidateStaleContextResets(sessionId, keptMsgs.length);
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
    [syncVisible, setOp, loadMessages, invalidateStaleContextResets],
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

  // ------------------------------------------------------------------
  // addContextReset -- add a new context reset point
  // ------------------------------------------------------------------

  const addContextReset = useCallback(() => {
    const sid = activeSessionIdRef.current;
    if (!sid) return;

    // Set: add a new reset point at the current message count.
    const msgs = getCachedMessages(sessionMessagesRef.current, sid);
    const idx = msgs.length;
    const existing = contextResetMapRef.current.get(sid) ?? [];
    // Avoid adding duplicate if the last point is already at this index.
    if (existing.length > 0 && existing[existing.length - 1] === idx) return;
    const updated = [...existing, idx];
    contextResetMapRef.current.set(sid, updated);
    setContextResetPoints(updated);

    // Persist to backend so it survives app restarts.
    invoke('session_set_context_reset', { sessionId: sid, index: idx })
      .catch((e) => console.error('[chat] failed to persist context reset:', e));
  }, []);

  // ------------------------------------------------------------------
  // addCompactPoint -- record a compaction point for display
  // ------------------------------------------------------------------

  const addCompactPoint = useCallback((info: Omit<CompactInfo, 'atIndex'>) => {
    const sid = activeSessionIdRef.current;
    if (!sid) return;

    const msgs = getCachedMessages(sessionMessagesRef.current, sid);
    const entry: CompactInfo = { ...info, atIndex: msgs.length };
    const existing = compactMapRef.current.get(sid) ?? [];
    const updated = [...existing, entry];
    compactMapRef.current.set(sid, updated);
    setCompactPoints(updated);
  }, []);

  return {
    messages: visibleMessages,
    isStreaming,
    isLoadingMessages,
    streamingSessionIds,
    activeRunId,
    error,
    opStatus,
    pendingEdit,
    toolResults: visibleToolResults,
    getStreamSegments,
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
    contextResetPoints,
    addContextReset,
    compactPoints,
    addCompactPoint,
    setOp,
  };
}
