// ---------------------------------------------------------------------------
// useChatStreaming -- ChatBus subscription, stream segments, tool results,
// and safety timeout for stuck sessions.
//
// Extracted from useChat.ts. Owns streamingSessionIds, activeRunId,
// visibleToolResults, streamSegsVersion state.
// ---------------------------------------------------------------------------

import { useState, useCallback, useEffect, useRef, type Dispatch, type SetStateAction, type MutableRefObject } from 'react';
import { startTransition } from 'react';
import { transport } from '../lib';
import type { Message, GeneratedImage } from '../types';
import { capSegments, capToolResults } from './streamCapping';
import {
  chatBusState,
  chatBusSubscribers,
  processedCancelledRuns,
  type ChatBusSubscriber,
} from './chatBus';
import { CHAT_STUCK_TIMEOUT_MS, hasSessionActivityTimedOut } from './chatActivity';
import { hasPendingRunForSession } from './chatRunState';
import {
  getCachedMessages,
  setCachedMessages,
  mergeSkillsFromCache,
} from './chatHelpers';
import {
  ensureStreamingAssistantMessage,
  finalizeStreamingAssistantMessage,
  mergeBackendMessagesPreservingLocalStreamState,
  streamingAssistantMessageId,
} from './chatStreamingMessages';
import {
  shouldDisplayStreamingAgent,
  type ToolResultRecord,
} from './chatStreamTypes';
import {
  applyGeneratedImageDelta,
  extractGeneratedImages,
  mergeGeneratedImages,
  upsertGeneratedImage,
} from '../lib/generatedImages';
import { mergeToolResultMetadata } from './toolResultMetadata';
import { upsertToolResultRecord, upsertToolResultSegment } from './toolResultUpdates';
import type { InterleavedSegment } from './useInterleavedSegments';
import type { ChatSharedRefs } from './chatSharedState';
import type { ChatOpStatus } from './useChat';

export interface UseChatStreamingReturn {
  streamingSessionIds: Set<string>;
  setStreamingSessionIds: Dispatch<SetStateAction<Set<string>>>;
  activeRunId: string | null;
  activeRunIdRef: MutableRefObject<string | null>;
  visibleToolResults: ToolResultRecord[];
  setVisibleToolResults: Dispatch<SetStateAction<ToolResultRecord[]>>;
  getStreamSegments: () => InterleavedSegment[] | null;
  streamSegsVersion: number;
  setStreamSegsVersion: Dispatch<SetStateAction<number>>;
}

export function useChatStreaming(
  refs: ChatSharedRefs,
  setOp: (status: ChatOpStatus) => void,
  setOpForSession: (sessionId: string, status: ChatOpStatus) => void,
  syncVisible: (sessionId: string) => void,
  updateStreamingGeneratedImages: (
    sessionId: string,
    updater: (images: GeneratedImage[]) => GeneratedImage[],
  ) => void,
  setVisibleMessages: Dispatch<SetStateAction<Message[]>>,
  setError: Dispatch<SetStateAction<string | null>>,
  markSessionActivity: (sessionId: string, at?: number) => void,
): UseChatStreamingReturn {
  const [streamingSessionIds, setStreamingSessionIds] = useState<Set<string>>(
    new Set(chatBusState.streamingSessions),
  );
  const [activeRunId, setActiveRunId] = useState<string | null>(null);

  // Stable ref for activeRunId so bus handler and safety timeout can read
  // the current value without re-subscribing on every state change.
  const activeRunIdRef = useRef<string | null>(null);

  const [visibleToolResults, setVisibleToolResults] = useState<ToolResultRecord[]>([]);

  // Counter bumped on structural changes (tool_result added) to force
  // re-renders in consuming components.
  const [streamSegsVersion, setStreamSegsVersion] = useState(0);

  const getStreamSegments = useCallback((): InterleavedSegment[] | null => {
    const sid = refs.activeSessionIdRef.current;
    if (!sid) return null;
    const segs = refs.streamSegsRef.current.get(sid);
    if (!segs || segs.length === 0) return null;
    // Only return segments when there are tool results or reasoning
    // (otherwise plain text handled by the bubble directly).
    if (segs.some((s) => s.type === 'tool_result' || s.type === 'reasoning')) return segs;
    return null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamSegsVersion, refs.activeSessionIdRef, refs.streamSegsRef]);

  // Subscribe to the chat bus on mount.
  useEffect(() => {
    setStreamingSessionIds(new Set(chatBusState.streamingSessions));

    const handler: ChatBusSubscriber = (event) => {
      if (event.type === 'started') {
        markSessionActivity(event.session_id);
        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        activeRunIdRef.current = event.run_id;
        setActiveRunId(event.run_id);
        // Clear tool results and stream segments for the new run.
        refs.toolResultsRef.current.set(event.session_id, []);
        refs.streamSegsRef.current.set(event.session_id, []);
        if (event.session_id === refs.activeSessionIdRef.current) {
          setVisibleToolResults([]);
        }
        console.log('[chat] run started, run_id =', event.run_id, 'session =', event.session_id);
      } else if (event.type === 'stream_delta') {
        if (!shouldDisplayStreamingAgent(event.agent_name, refs.rootAgentNamesRef.current)) {
          return;
        }
        const sid = event.session_id;
        markSessionActivity(sid);
        // Append to event-ordered segments (text segment).
        const segs = refs.streamSegsRef.current.get(sid);
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
        setCachedMessages(refs.sessionMessagesRef.current, sid, (prev) => {
          const streamingId = streamingAssistantMessageId(sid);
          const lastIdx = prev.findIndex((m) => m.id === streamingId);
          if (lastIdx >= 0) {
            const updated = [...prev];
            const existing = updated[lastIdx];
            // When first content delta arrives, mark reasoning as done.
            const meta = { ...(existing.metadata || {}) };
            if (meta._reasoningStartTs && !meta._reasoningDoneTs) {
              meta._reasoningDoneTs = Date.now();
              meta._reasoningDurationMs =
                (meta._reasoningDoneTs as number) - (meta._reasoningStartTs as number);
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
        if (!shouldDisplayStreamingAgent(event.agent_name, refs.rootAgentNamesRef.current)) {
          return;
        }
        const sid = event.session_id;
        markSessionActivity(sid);
        console.log(
          `[chat] stream_reasoning_delta: session=${sid}, len=${event.content.length}`,
        );
        // Push/extend a reasoning segment in event-ordered segments.
        const segs = refs.streamSegsRef.current.get(sid);
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
        setCachedMessages(refs.sessionMessagesRef.current, sid, (prev) => {
          const streamingId = streamingAssistantMessageId(sid);
          const lastIdx = prev.findIndex((m) => m.id === streamingId);
          if (lastIdx >= 0) {
            const updated = [...prev];
            const existing = updated[lastIdx];
            const meta = { ...(existing.metadata || {}) };
            meta.reasoning_content =
              ((meta.reasoning_content as string) || '') + event.content;
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
      } else if (event.type === 'stream_image_delta') {
        if (!shouldDisplayStreamingAgent(event.agent_name, refs.rootAgentNamesRef.current)) {
          return;
        }
        const sid = event.session_id;
        markSessionActivity(sid);
        updateStreamingGeneratedImages(sid, (images) =>
          applyGeneratedImageDelta(images, {
            index: event.index,
            mime_type: event.mime_type,
            partial_data: event.partial_data,
          }),
        );
      } else if (event.type === 'stream_image_complete') {
        if (!shouldDisplayStreamingAgent(event.agent_name, refs.rootAgentNamesRef.current)) {
          return;
        }
        const sid = event.session_id;
        markSessionActivity(sid);
        updateStreamingGeneratedImages(sid, (images) =>
          upsertGeneratedImage(images, {
            index: event.index,
            mime_type: event.mime_type,
            data: event.data,
          }),
        );
      } else if (event.type === 'tool_start') {
        if (!shouldDisplayStreamingAgent(event.agent_name, refs.rootAgentNamesRef.current)) {
          return;
        }
        const sid = event.session_id;
        markSessionActivity(sid);
        const record: ToolResultRecord = {
          name: event.name,
          arguments: event.input_preview,
          success: true,
          durationMs: 0,
          resultPreview: '',
          state: 'running',
        };
        const existing = refs.toolResultsRef.current.get(sid) ?? [];
        const nextToolResults = upsertToolResultRecord(existing, record);
        const cappedResults = capToolResults(nextToolResults.records);
        refs.toolResultsRef.current.set(sid, cappedResults);
        if (sid === refs.activeSessionIdRef.current) {
          setVisibleToolResults(cappedResults);
        }
        const segs = refs.streamSegsRef.current.get(sid) ?? [];
        const nextSegments = upsertToolResultSegment(segs, record);
        refs.streamSegsRef.current.set(sid, capSegments(nextSegments.segments));
        setCachedMessages(refs.sessionMessagesRef.current, sid, (prev) =>
          ensureStreamingAssistantMessage(prev, sid),
        );
        setStreamSegsVersion((v) => v + 1);
        syncVisible(sid);
      } else if (event.type === 'complete') {
        const payload = event.payload;
        // Resolve session: prefer payload (always available from backend),
        // fall back to bus mapping for older event formats.
        const sessionId =
          payload.session_id || chatBusState.runToSession[payload.run_id] || undefined;
        if (sessionId) {
          markSessionActivity(sessionId);
        }
        const sessionStillActive = sessionId
          ? hasPendingRunForSession(chatBusState, sessionId)
          : false;
        console.log(
          `[chat] complete: run_id=${payload.run_id}, session=${sessionId}, opStatus=${refs.opStatusRef.current}`,
        );

        if (sessionStillActive) {
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          return;
        }

        if (sessionId) {
          // Merge streaming content with backend messages.
          (async () => {
            const streamingId = streamingAssistantMessageId(sessionId);
            const cachedMessages = getCachedMessages(
              refs.sessionMessagesRef.current,
              sessionId,
            );
            const streamingMsg = cachedMessages.find((m) => m.id === streamingId);
            try {
              const msgs = await transport.invoke<Message[]>(
                'session_get_messages',
                { sessionId },
              );

              // Preserve skill tags from optimistic user messages.
              const mergedMsgs = mergeSkillsFromCache(
                msgs,
                refs.sessionMessagesRef.current,
                sessionId,
              );

              // Grab the accumulated streaming content before overwriting.
              const accumulatedContent = streamingMsg?.content ?? '';
              const snapToolResults = refs.toolResultsRef.current.get(sessionId);

              if (mergedMsgs.length > 0) {
                const lastMsg = mergedMsgs[mergedMsgs.length - 1];
                if (lastMsg.role === 'assistant') {
                  // Merge streaming reasoning metadata.
                  const streamMeta = streamingMsg?.metadata;
                  const mergedMeta = { ...(lastMsg.metadata || {}) };
                  if (streamMeta) {
                    if (streamMeta._reasoningDurationMs) {
                      mergedMeta._reasoningDurationMs = streamMeta._reasoningDurationMs;
                    }
                    if (streamMeta._reasoningDoneTs) {
                      mergedMeta._reasoningDoneTs = streamMeta._reasoningDoneTs;
                    }
                    if (!mergedMeta.reasoning_content && streamMeta.reasoning_content) {
                      mergedMeta.reasoning_content = streamMeta.reasoning_content;
                    }
                    const mergedImages = mergeGeneratedImages(
                      extractGeneratedImages(mergedMeta),
                      extractGeneratedImages(streamMeta),
                    );
                    if (mergedImages.length > 0) {
                      mergedMeta.generated_images = mergedImages;
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
                    mergedMsgs[mergedMsgs.length - 1] = {
                      ...lastMsg,
                      content: accumulatedContent,
                      metadata: mergedMeta,
                    };
                  } else if (
                    Object.keys(mergedMeta).length >
                    Object.keys(lastMsg.metadata || {}).length
                  ) {
                    mergedMsgs[mergedMsgs.length - 1] = {
                      ...lastMsg,
                      metadata: mergedMeta,
                    };
                  }
                }
              }

              const localMessagesWithoutCompletedStream = getCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
              ).filter((message) => message.id !== streamingId);
              const mergedWithLocalState =
                mergeBackendMessagesPreservingLocalStreamState(
                  mergedMsgs,
                  localMessagesWithoutCompletedStream,
                );
              setCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
                mergedWithLocalState,
              );
              if (refs.activeSessionIdRef.current === sessionId) {
                startTransition(() => {
                  setVisibleMessages(mergedWithLocalState);
                });
              }
            } catch (e) {
              console.error('[chat] complete: failed to reload messages:', e);
              // Fallback: synthesize the assistant message in cache.
              setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) => {
                const sid = `streaming-${sessionId}`;
                const filtered = prev.filter((m) => m.id !== sid);
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
                  tokens: {
                    input: payload.input_tokens,
                    output: payload.output_tokens,
                  },
                  cost: payload.cost_usd,
                  context_window: payload.context_window,
                  metadata: {
                    tool_results:
                      mergeToolResultMetadata(
                        [],
                        refs.toolResultsRef.current.get(sessionId),
                      ) ?? [],
                    ...(extractGeneratedImages(streamingMsg?.metadata).length > 0
                      ? {
                          generated_images: extractGeneratedImages(
                            streamingMsg?.metadata,
                          ),
                        }
                      : {}),
                  },
                };
                if (filtered.some((m) => m.id === newMsg.id)) return filtered;
                return [...filtered, newMsg];
              });
              syncVisible(sessionId);
            } finally {
              if (!sessionStillActive) {
                setOpForSession(sessionId, 'idle');
              }
            }
          })();
        } else {
          // No session to reload -- transition immediately.
          if (refs.opStatusRef.current !== 'idle') {
            setOp('idle');
          }
        }

        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        if (sessionId && !sessionStillActive) {
          refs.toolResultsRef.current.delete(sessionId);
          refs.streamSegsRef.current.delete(sessionId);
        }
        if (activeRunIdRef.current === payload.run_id) {
          activeRunIdRef.current = null;
          setActiveRunId(null);
        }
        setError(null);
      } else if (event.type === 'error') {
        const payload = event.payload;
        const sessionId =
          payload.session_id || chatBusState.runToSession[payload.run_id] || undefined;
        if (sessionId) {
          markSessionActivity(sessionId);
        }
        const sessionStillActive = sessionId
          ? hasPendingRunForSession(chatBusState, sessionId)
          : false;
        const isCancelled = payload.error === 'Cancelled';

        if (sessionStillActive) {
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          return;
        }

        // Deduplicate cancel events.
        if (isCancelled && processedCancelledRuns.has(payload.run_id)) {
          setStreamingSessionIds(new Set(chatBusState.streamingSessions));
          if (activeRunIdRef.current === payload.run_id) {
            activeRunIdRef.current = null;
            setActiveRunId(null);
          }
          return;
        }
        if (isCancelled) {
          processedCancelledRuns.add(payload.run_id);
          setTimeout(() => processedCancelledRuns.delete(payload.run_id), 30_000);
        }

        if (sessionId) {
          if (isCancelled) {
            // Stop/cancel: preserve streamed content by finalizing the
            // streaming message.
            const snapToolResults = refs.toolResultsRef.current.get(sessionId);
            const cancelledMessageId = `cancelled-${payload.run_id}`;

            setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) => {
              return finalizeStreamingAssistantMessage(
                prev,
                sessionId,
                cancelledMessageId,
                snapToolResults,
              );
            });
            syncVisible(sessionId);

            // Reload from backend so the cache has real backend IDs.
            (async () => {
              try {
                const backendMsgs = await transport.invoke<Message[]>(
                  'session_get_messages',
                  { sessionId },
                );
                const mergedBack = mergeSkillsFromCache(
                  backendMsgs,
                  refs.sessionMessagesRef.current,
                  sessionId,
                );
                const currentCached = getCachedMessages(
                  refs.sessionMessagesRef.current,
                  sessionId,
                );
                const merged = mergeBackendMessagesPreservingLocalStreamState(
                  mergedBack,
                  currentCached,
                );
                setCachedMessages(refs.sessionMessagesRef.current, sessionId, merged);
                if (refs.activeSessionIdRef.current === sessionId) {
                  startTransition(() => setVisibleMessages(merged));
                }
              } catch (e) {
                console.error('[chat] cancel: failed to reload messages:', e);
              }
            })();
          } else {
            // Non-cancel error: preserve streamed content.
            const snapToolResultsErr = refs.toolResultsRef.current.get(sessionId);
            const errorMessageId = `error-${payload.run_id || Date.now()}`;

            setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) => {
              return finalizeStreamingAssistantMessage(
                prev,
                sessionId,
                errorMessageId,
                snapToolResultsErr,
              );
            });
            syncVisible(sessionId);

            // Async reload: merge backend messages with the preserved error message.
            (async () => {
              try {
                const backendMsgs = await transport.invoke<Message[]>(
                  'session_get_messages',
                  { sessionId },
                );
                const mergedBack = mergeSkillsFromCache(
                  backendMsgs,
                  refs.sessionMessagesRef.current,
                  sessionId,
                );
                const currentCached = getCachedMessages(
                  refs.sessionMessagesRef.current,
                  sessionId,
                );
                const final_ = mergeBackendMessagesPreservingLocalStreamState(
                  mergedBack,
                  currentCached,
                );
                setCachedMessages(refs.sessionMessagesRef.current, sessionId, final_);
                if (refs.activeSessionIdRef.current === sessionId) {
                  startTransition(() => setVisibleMessages(final_));
                }
              } catch (reloadErr) {
                console.error(
                  '[chat] error handler: failed to reload messages:',
                  reloadErr,
                );
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
          if (!sessionId || sessionId === refs.activeSessionIdRef.current) {
            setError(payload.error);
          }
        }

        // Return to idle on error too.
        if (sessionId) {
          if (!sessionStillActive) {
            setOpForSession(sessionId, 'idle');
            refs.toolResultsRef.current.delete(sessionId);
            refs.streamSegsRef.current.delete(sessionId);
          }
        } else if (refs.opStatusRef.current !== 'idle') {
          setOp('idle');
        }
      } else if (event.type === 'tool_result') {
        // Accumulate tool results for inline card rendering.
        const sid = event.session_id;
        markSessionActivity(sid);
        const record: ToolResultRecord = {
          name: event.name,
          arguments: event.input_preview,
          success: event.success,
          durationMs: event.duration_ms,
          resultPreview: event.result_preview,
          state: 'completed',
          urlMeta: event.url_meta,
          metadata: event.metadata,
        };
        const existing = refs.toolResultsRef.current.get(sid) ?? [];
        const nextToolResults = upsertToolResultRecord(existing, record);
        const cappedResults = capToolResults(nextToolResults.records);
        refs.toolResultsRef.current.set(sid, cappedResults);
        if (sid === refs.activeSessionIdRef.current) {
          setVisibleToolResults(cappedResults);
        }
        // Push or replace a tool_result segment and bump version to force re-render.
        const segs = refs.streamSegsRef.current.get(sid) ?? [];
        let preparedSegs = segs;
        const lastSeg = segs[segs.length - 1];
        if (lastSeg && lastSeg.type === 'reasoning' && lastSeg.isStreaming) {
          preparedSegs = [...segs];
          preparedSegs[preparedSegs.length - 1] = {
            ...lastSeg,
            isStreaming: false,
            durationMs: lastSeg._startTs
              ? Date.now() - lastSeg._startTs
              : lastSeg.durationMs,
          };
        }
        const nextSegments = upsertToolResultSegment(preparedSegs, record);
        refs.streamSegsRef.current.set(sid, capSegments(nextSegments.segments));
        setCachedMessages(refs.sessionMessagesRef.current, sid, (prev) =>
          ensureStreamingAssistantMessage(prev, sid),
        );
        setStreamSegsVersion((v) => v + 1);
        syncVisible(sid);
      }
    };

    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    markSessionActivity,
    setOp,
    setOpForSession,
    syncVisible,
    updateStreamingGeneratedImages,
    setVisibleMessages,
    setError,
  ]);

  // ------------------------------------------------------------------
  // Safety timeout: if opStatus stays non-idle for too long without any
  // bus activity, force-reset to idle so the session is never permanently
  // stuck.
  // ------------------------------------------------------------------

  const opStatus = refs.opStatusRef.current;

  useEffect(() => {
    if (opStatus === 'idle') return;

    const SAFETY_POLL_MS = 15_000;
    const checkForStuckSession = () => {
      const sid = refs.activeSessionIdRef.current;
      if (!sid || refs.opStatusRef.current === 'idle') {
        return;
      }

      const lastActivityAt = refs.sessionActivityRef.current.get(sid);
      if (
        !hasSessionActivityTimedOut(lastActivityAt, Date.now(), CHAT_STUCK_TIMEOUT_MS)
      ) {
        return;
      }

      console.warn(
        `[chat] safety timeout: session=${sid} opStatus=${refs.opStatusRef.current} inactive for ${CHAT_STUCK_TIMEOUT_MS}ms, forcing idle`,
      );
      setOpForSession(sid, 'idle');
      chatBusState.streamingSessions.delete(sid);
      setStreamingSessionIds(new Set(chatBusState.streamingSessions));
      if (
        activeRunIdRef.current &&
        chatBusState.runToSession[activeRunIdRef.current] === sid
      ) {
        activeRunIdRef.current = null;
        setActiveRunId(null);
      }
    };

    checkForStuckSession();
    const timer = setInterval(checkForStuckSession, SAFETY_POLL_MS);
    return () => clearInterval(timer);
  }, [opStatus, setOpForSession, refs.activeSessionIdRef, refs.opStatusRef, refs.sessionActivityRef]);

  return {
    streamingSessionIds,
    setStreamingSessionIds,
    activeRunId,
    activeRunIdRef,
    visibleToolResults,
    setVisibleToolResults,
    getStreamSegments,
    streamSegsVersion,
    setStreamSegsVersion,
  };
}
