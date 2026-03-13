// Custom hook for chat functionality -- per-session streaming state.
//
// Tauri event listeners are registered in a module-level singleton so
// React StrictMode double-mount never creates duplicate handlers.

import { useState, useCallback, useEffect, useRef, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  Message,
  ChatStarted,
  ChatCompletePayload,
  ChatErrorPayload,
  ChatStartedPayload,
  ProgressPayload,
} from '../types';

interface UseChatReturn {
  messages: Message[];
  isStreaming: boolean;
  isLoadingMessages: boolean;
  streamingSessionIds: Set<string>;
  activeRunId: string | null;
  error: string | null;
  sendMessage: (message: string, sessionId: string, providerId?: string) => Promise<ChatStarted | null>;
  cancelRun: () => Promise<void>;
  loadMessages: (sessionId: string) => Promise<void>;
  clearMessages: () => void;
}

// ---------------------------------------------------------------------------
// Module-level singleton bus
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
let chatUnlistenFns: UnlistenFn[] = [];

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

  // Listen for chat:progress events to forward stream_delta events.
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
// Hook
// ---------------------------------------------------------------------------

export function useChat(activeSessionId: string | null): UseChatReturn {
  const [messages, setMessages] = useState<Message[]>([]);
  const [streamingSessionIds, setStreamingSessionIds] = useState<Set<string>>(
    new Set(chatBusState.streamingSessions),
  );
  const [error, setError] = useState<string | null>(null);
  const [isLoadingMessages, setIsLoadingMessages] = useState(false);
  // Track the active run_id for the currently viewed session (for cancel).
  const activeRunIdRef = useRef<string | null>(null);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);

  // Track which session's messages are currently displayed.
  const loadedSessionRef = useRef<string | null>(null);

  // Subscribe to the chat bus on mount.
  useEffect(() => {
    // Sync streaming state in case events fired before mount.
    setStreamingSessionIds(new Set(chatBusState.streamingSessions));

    const handler: ChatBusSubscriber = (event) => {
      if (event.type === 'started') {
        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        // Always track the most-recently started run_id.
        activeRunIdRef.current = event.run_id;
        setActiveRunId(event.run_id);
        console.log('[chat] run started, run_id =', event.run_id, 'session =', event.session_id);
      } else if (event.type === 'stream_delta') {
        // Append incremental text to the streaming assistant message.
        if (event.session_id === loadedSessionRef.current) {
          setMessages((prev) => {
            const streamingId = `streaming-${event.session_id}`;
            const lastIdx = prev.findIndex((m) => m.id === streamingId);
            if (lastIdx >= 0) {
              // Append to existing streaming message.
              const updated = [...prev];
              updated[lastIdx] = {
                ...updated[lastIdx],
                content: updated[lastIdx].content + event.content,
              };
              return updated;
            }
            // Create a new streaming message.
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
        }
      } else if (event.type === 'complete') {
        const payload = event.payload;
        const sessionId = chatBusState.runToSession[payload.run_id];

        if (sessionId && sessionId === loadedSessionRef.current) {
          setMessages((prev) => {
            // Remove the streaming placeholder if present.
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
            // Avoid duplicate: don't append if identical run_id already present.
            if (filtered.some((m) => m.id === newMsg.id)) return filtered;
            return [...filtered, newMsg];
          });
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

        // Remove the streaming placeholder on error too.
        if (sessionId && sessionId === loadedSessionRef.current) {
          setMessages((prev) => {
            const streamingId = `streaming-${sessionId}`;
            return prev.filter((m) => m.id !== streamingId);
          });
        }

        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        if (activeRunIdRef.current === payload.run_id) {
          activeRunIdRef.current = null;
          setActiveRunId(null);
        }
        // Do not surface "Cancelled" as an error -- it was intentional.
        if (payload.error !== 'Cancelled') {
          if (!sessionId || sessionId === loadedSessionRef.current) {
            setError(payload.error);
          }
        }
      }
    };

    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  }, []);

  const sendMessage = useCallback(
    async (message: string, sessionId: string, providerId?: string): Promise<ChatStarted | null> => {
      setError(null);

      // Register the session as the loaded one so chat:complete can append
      // the result even if loadMessages was never called (e.g. fresh session).
      loadedSessionRef.current = sessionId;

      // Optimistically add user message.
      setMessages((prev) => [
        ...prev,
        {
          id: `user-${Date.now()}`,
          role: 'user' as const,
          content: message,
          timestamp: new Date().toISOString(),
          tool_calls: [],
        },
      ]);

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
        return null;
      }
    },
    [],
  );

  const loadMessages = useCallback(async (sessionId: string) => {
    // Always update the loaded-session ref so that incoming chat:complete /
    // stream_delta events for this session are correctly applied.
    loadedSessionRef.current = sessionId;

    // Wrap state mutations in startTransition so React treats them as
    // low-priority updates.  The sidebar's activeSessionId re-render
    // (high priority) commits and paints first; the chat panel update
    // renders separately afterwards, preventing the sidebar focus from
    // appearing stuck on the old session.
    startTransition(() => {
      setMessages([]);
      setIsLoadingMessages(true);
    });

    try {
      const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
      // Only set if we're still loading the same session.
      if (loadedSessionRef.current === sessionId) {
        // If a run is currently streaming for this session, preserve the
        // streaming placeholder message so the user sees incremental text.
        const streamingId = `streaming-${sessionId}`;
        startTransition(() => {
          setMessages((prev) => {
            const existingStreaming = prev.find((m) => m.id === streamingId);
            if (existingStreaming) {
              return [...msgs, existingStreaming];
            }
            return msgs;
          });
        });
      }
    } catch (e) {
      setError(String(e));
    } finally {
      if (loadedSessionRef.current === sessionId) {
        startTransition(() => {
          setIsLoadingMessages(false);
        });
      }
    }
  }, []);

  const clearMessages = useCallback(() => {
    loadedSessionRef.current = null;
    setMessages([]);
    setError(null);
  }, []);

  const isStreaming = activeSessionId ? streamingSessionIds.has(activeSessionId) : false;

  const cancelRun = useCallback(async () => {
    console.log('[chat] cancelRun called');
    console.log('[chat] activeRunIdRef.current =', activeRunIdRef.current);
    console.log('[chat] chatBusState.pendingRuns =', [...chatBusState.pendingRuns]);
    console.log('[chat] loadedSessionRef.current =', loadedSessionRef.current);

    // Prefer the tracked run_id ref; fall back to any pending run for the
    // currently loaded session as a safety net.
    let runId = activeRunIdRef.current;
    if (!runId) {
      // Find a pending run belonging to the currently loaded session.
      const sessionId = loadedSessionRef.current;
      if (sessionId) {
        runId = [...chatBusState.pendingRuns].find(
          (rid) => chatBusState.runToSession[rid] === sessionId,
        ) ?? null;
      }
    }
    if (!runId) {
      console.warn('[chat] cancelRun: no active run found, aborting');
      return;
    }
    console.log('[chat] invoking chat_cancel with runId =', runId);
    try {
      // Tauri v2 #[tauri::command] auto-renames camelCase -> snake_case,
      // so { runId } maps to the Rust run_id parameter (same as sessionId -> session_id).
      await invoke('chat_cancel', { runId });
      console.log('[chat] chat_cancel invoke succeeded');
    } catch (e) {
      console.error('[chat] chat_cancel invoke failed:', e);
    }
  }, []);

  return {
    messages,
    isStreaming,
    isLoadingMessages,
    streamingSessionIds,
    activeRunId,
    error,
    sendMessage,
    cancelRun,
    loadMessages,
    clearMessages,
  };
}
