// ---------------------------------------------------------------------------
// useChatMessages -- message cache management, loading, and sync.
//
// Extracted from useChat.ts. Owns visibleMessages state and the
// loadMessages / clearMessages operations.
// ---------------------------------------------------------------------------

import { useState, useCallback, type Dispatch, type SetStateAction } from 'react';
import { startTransition } from 'react';
import { transport } from '../lib';
import type { Message } from '../types';
import type { GeneratedImage } from '../types';
import {
  getCachedMessages,
  setCachedMessages,
  mergeSkillsFromCache,
} from './chatHelpers';
import { extractGeneratedImages } from '../lib/generatedImages';
import type { ChatSharedRefs } from './chatSharedState';
import type { ChatOpStatus, PendingEdit } from './useChat';

export interface UseChatMessagesReturn {
  visibleMessages: Message[];
  setVisibleMessages: Dispatch<SetStateAction<Message[]>>;
  isLoadingMessages: boolean;
  setIsLoadingMessages: Dispatch<SetStateAction<boolean>>;
  error: string | null;
  setError: Dispatch<SetStateAction<string | null>>;
  loadMessages: (sessionId: string) => Promise<void>;
  clearMessages: () => void;
  syncVisible: (sessionId: string) => void;
  updateStreamingGeneratedImages: (
    sessionId: string,
    updater: (images: GeneratedImage[]) => GeneratedImage[],
  ) => void;
}

export function useChatMessages(
  refs: ChatSharedRefs,
  setOp: (status: ChatOpStatus) => void,
  setPendingEdit: Dispatch<SetStateAction<PendingEdit | null>>,
  setContextResetPoints: Dispatch<SetStateAction<number[]>>,
): UseChatMessagesReturn {
  const [visibleMessages, setVisibleMessages] = useState<Message[]>([]);
  const [isLoadingMessages, setIsLoadingMessages] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Flush visible messages from cache for the given session.
  const syncVisible = useCallback((sessionId: string) => {
    if (sessionId === refs.activeSessionIdRef.current) {
      setVisibleMessages(
        getCachedMessages(refs.sessionMessagesRef.current, sessionId),
      );
    }
  }, [refs.activeSessionIdRef, refs.sessionMessagesRef]);

  const updateStreamingGeneratedImages = useCallback((
    sessionId: string,
    updater: (images: GeneratedImage[]) => GeneratedImage[],
  ) => {
    setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) => {
      const streamingId = `streaming-${sessionId}`;
      const messageIndex = prev.findIndex((message) => message.id === streamingId);
      if (messageIndex >= 0) {
        const updated = [...prev];
        const existing = updated[messageIndex];
        const metadata = { ...(existing.metadata || {}) };
        metadata.generated_images = updater(extractGeneratedImages(metadata));
        updated[messageIndex] = {
          ...existing,
          metadata,
        };
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
            generated_images: updater([]),
          },
        } as Message,
      ];
    });
    syncVisible(sessionId);
  }, [refs.sessionMessagesRef, syncVisible]);

  const loadMessages = useCallback(async (sessionId: string) => {
    refs.activeSessionIdRef.current = sessionId;

    const cachedMsgs = getCachedMessages(refs.sessionMessagesRef.current, sessionId);
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
        transport.invoke<Message[]>('session_get_messages', { sessionId }),
        transport.invoke<number | null>('session_get_context_reset', { sessionId }),
      ]);

      // Restore persisted context reset points.
      if (resetIdx != null) {
        const points = [resetIdx];
        refs.contextResetMapRef.current.set(sessionId, points);
        if (refs.activeSessionIdRef.current === sessionId) {
          setContextResetPoints(points);
        }
      } else if (!refs.contextResetMapRef.current.has(sessionId)) {
        // No persisted reset and no in-memory entry: ensure clean state.
        refs.contextResetMapRef.current.set(sessionId, []);
        if (refs.activeSessionIdRef.current === sessionId) {
          setContextResetPoints([]);
        }
      }

      // Preserve skill tags from cached messages.
      const mergedMsgs = mergeSkillsFromCache(
        msgs,
        refs.sessionMessagesRef.current,
        sessionId,
      );
      console.log(
        `[chat] loadMessages: got ${mergedMsgs.length} messages for session=${sessionId}, active=${refs.activeSessionIdRef.current}`,
      );
      if (refs.activeSessionIdRef.current === sessionId) {
        const streamingId = `streaming-${sessionId}`;
        // Re-read from cache (may have been updated by sendMessage in the meantime).
        const currentCached = getCachedMessages(refs.sessionMessagesRef.current, sessionId);
        const existingStreaming = currentCached.find((m) => m.id === streamingId);

        // Preserve optimistic user messages (id starts with "user-") that
        // exist in the cache but are not yet in the backend response.
        const backendIds = new Set(mergedMsgs.map((m) => m.id));
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

        setCachedMessages(refs.sessionMessagesRef.current, sessionId, merged);
        startTransition(() => {
          setVisibleMessages(merged);
        });
      } else {
        console.log(
          `[chat] loadMessages: session mismatch, skipping visible update (active=${refs.activeSessionIdRef.current}, requested=${sessionId})`,
        );
        setCachedMessages(refs.sessionMessagesRef.current, sessionId, mergedMsgs);
      }
    } catch (e) {
      console.error('[chat] loadMessages failed:', e);
      setError(String(e));
    } finally {
      if (refs.activeSessionIdRef.current === sessionId) {
        startTransition(() => {
          setIsLoadingMessages(false);
        });
      }
    }
  }, [refs.activeSessionIdRef, refs.sessionMessagesRef, refs.contextResetMapRef, setContextResetPoints]);

  const clearMessages = useCallback(() => {
    const sid = refs.activeSessionIdRef.current;
    if (sid) {
      refs.sessionMessagesRef.current.delete(sid);
    }
    refs.activeSessionIdRef.current = null;
    setVisibleMessages([]);
    setError(null);
    setPendingEdit(null);
    setOp('idle');
  }, [refs.activeSessionIdRef, refs.sessionMessagesRef, setOp, setPendingEdit]);

  return {
    visibleMessages,
    setVisibleMessages,
    isLoadingMessages,
    setIsLoadingMessages,
    error,
    setError,
    loadMessages,
    clearMessages,
    syncVisible,
    updateStreamingGeneratedImages,
  };
}
