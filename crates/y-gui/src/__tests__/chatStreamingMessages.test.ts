import { describe, expect, it } from 'vitest';

import { ensureStreamingAssistantMessage } from '../hooks/chatStreamingMessages';
import type { Message } from '../types';

describe('ensureStreamingAssistantMessage', () => {
  it('adds a streaming assistant placeholder when a non-text event arrives first', () => {
    const messages: Message[] = [
      {
        id: 'user-1',
        role: 'user',
        content: 'Run the tests',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
    ];

    const updated = ensureStreamingAssistantMessage(
      messages,
      'session-1',
      '2026-04-24T00:00:01.000Z',
    );

    expect(updated).toHaveLength(2);
    expect(updated[1]).toMatchObject({
      id: 'streaming-session-1',
      role: 'assistant',
      content: '',
      timestamp: '2026-04-24T00:00:01.000Z',
      tool_calls: [],
      _streaming: true,
    });
  });

  it('keeps an existing streaming assistant message instead of duplicating it', () => {
    const existing = {
      id: 'streaming-session-1',
      role: 'assistant',
      content: 'partial answer',
      timestamp: '2026-04-24T00:00:01.000Z',
      tool_calls: [],
      _streaming: true,
    } as Message;

    const updated = ensureStreamingAssistantMessage(
      [existing],
      'session-1',
      '2026-04-24T00:00:02.000Z',
    );

    expect(updated).toEqual([existing]);
  });
});
