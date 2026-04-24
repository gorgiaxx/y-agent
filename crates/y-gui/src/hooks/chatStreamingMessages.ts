import type { Message } from '../types';

export function ensureStreamingAssistantMessage(
  messages: Message[],
  sessionId: string,
  timestamp = new Date().toISOString(),
): Message[] {
  const streamingId = `streaming-${sessionId}`;
  if (messages.some((message) => message.id === streamingId)) {
    return messages;
  }

  return [
    ...messages,
    {
      id: streamingId,
      role: 'assistant' as const,
      content: '',
      timestamp,
      tool_calls: [],
      _streaming: true,
    } as Message,
  ];
}
