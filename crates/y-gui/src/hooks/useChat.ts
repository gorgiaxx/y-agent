// Custom hook for chat functionality.

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Message, ChatStarted, ChatCompletePayload, ChatErrorPayload } from '../types';

interface UseChatReturn {
  messages: Message[];
  isStreaming: boolean;
  error: string | null;
  sendMessage: (message: string, sessionId: string | null) => Promise<ChatStarted | null>;
  loadMessages: (sessionId: string) => Promise<void>;
  clearMessages: () => void;
}

export function useChat(): UseChatReturn {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Listen to Tauri events for streaming.
  useEffect(() => {
    const unlisten: (() => void)[] = [];

    const setup = async () => {
      const u1 = await listen<ChatCompletePayload>('chat:complete', (event) => {
        const payload = event.payload;
        setMessages((prev) => [
          ...prev,
          {
            id: `assistant-${Date.now()}`,
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
          },
        ]);
        setIsStreaming(false);
      });
      unlisten.push(u1);

      const u2 = await listen<ChatErrorPayload>('chat:error', (event) => {
        setError(event.payload.error);
        setIsStreaming(false);
      });
      unlisten.push(u2);
    };

    setup();
    return () => unlisten.forEach((u) => u());
  }, []);

  const sendMessage = useCallback(
    async (message: string, sessionId: string | null): Promise<ChatStarted | null> => {
      setError(null);
      setIsStreaming(true);

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
        });
        return result;
      } catch (e) {
        setError(String(e));
        setIsStreaming(false);
        return null;
      }
    },
    [],
  );

  const loadMessages = useCallback(async (sessionId: string) => {
    try {
      const msgs = await invoke<Message[]>('session_get_messages', { sessionId });
      setMessages(msgs);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const clearMessages = useCallback(() => {
    setMessages([]);
    setError(null);
  }, []);

  return { messages, isStreaming, error, sendMessage, loadMessages, clearMessages };
}
