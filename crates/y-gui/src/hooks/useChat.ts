// Custom hook for chat functionality -- per-session streaming state.
//
// Tauri event listeners are registered in a module-level singleton so
// React StrictMode double-mount never creates duplicate handlers.

import { useState, useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  Message,
  ChatStarted,
  ChatCompletePayload,
  ChatErrorPayload,
  ChatStartedPayload,
} from '../types';

interface UseChatReturn {
  messages: Message[];
  isStreaming: boolean;
  streamingSessionIds: Set<string>;
  activeRunId: string | null;
  error: string | null;
  sendMessage: (message: string, sessionId: string) => Promise<ChatStarted | null>;
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
  | { type: 'error'; payload: ChatErrorPayload };

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
      } else if (event.type === 'complete') {
        const payload = event.payload;
        const sessionId = chatBusState.runToSession[payload.run_id];

        if (sessionId && sessionId === loadedSessionRef.current) {
          setMessages((prev) => {
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
              tokens: { input: payload.input_tokens, output: payload.output_tokens },
              cost: payload.cost_usd,
              context_window: payload.context_window,
            };
            // Avoid duplicate: don't append if identical run_id already present.
            if (prev.some((m) => m.id === newMsg.id)) return prev;
            return [...prev, newMsg];
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
    async (message: string, sessionId: string): Promise<ChatStarted | null> => {
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
        const result = await invoke<ChatStarted>('chat_send', { message, sessionId });
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
    // Do not reload if a run is currently in progress for this session --
    // the optimistic messages + chat:complete listener handle the display.
    const hasActiveRun = [...chatBusState.pendingRuns].some(
      (rid) => chatBusState.runToSession[rid] === sessionId,
    );
    if (hasActiveRun) return;

    loadedSessionRef.current = sessionId;
    try {
      const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
      // Only set if we're still loading the same session.
      if (loadedSessionRef.current === sessionId) {
        setMessages(msgs);
      }
    } catch (e) {
      setError(String(e));
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
    streamingSessionIds,
    activeRunId,
    error,
    sendMessage,
    cancelRun,
    loadMessages,
    clearMessages,
  };
}
