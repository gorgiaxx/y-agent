import { describe, expect, it } from 'vitest';

import {
  ensureStreamingAssistantMessage,
  finalizeStreamingAssistantMessage,
  isLiveStreamingAssistantMessage,
  mergeBackendMessagesPreservingLocalStreamState,
} from '../hooks/chatStreamingMessages';
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

  it('preserves a cancelled streaming message that only has tool results', () => {
    const messages: Message[] = [
      {
        id: 'user-1',
        role: 'user',
        content: 'Inspect the repo',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'streaming-session-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        _streaming: true,
      } as Message,
    ];

    const updated = finalizeStreamingAssistantMessage(
      messages,
      'session-1',
      'cancelled-run-1',
      [
        {
          name: 'FileRead',
          arguments: JSON.stringify({ path: '/tmp/source.rs' }),
          success: true,
          durationMs: 42,
          resultPreview: 'fn main() {}',
        },
      ],
    );

    expect(updated).toHaveLength(2);
    expect(updated[1]).toMatchObject({
      id: 'cancelled-run-1',
      role: 'assistant',
      content: '',
      metadata: {
        tool_results: [
          {
            name: 'FileRead',
            arguments: JSON.stringify({ path: '/tmp/source.rs' }),
            success: true,
            duration_ms: 42,
            result_preview: 'fn main() {}',
          },
        ],
      },
    });
    expect(updated[1]).not.toHaveProperty('_streaming');
  });

  it('preserves an in-flight tool start when cancellation happens before the result', () => {
    const messages: Message[] = [
      {
        id: 'streaming-session-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        _streaming: true,
      } as Message,
    ];

    const updated = finalizeStreamingAssistantMessage(
      messages,
      'session-1',
      'cancelled-run-1',
      [
        {
          name: 'ShellExec',
          arguments: JSON.stringify({ command: 'cargo test' }),
          success: true,
          durationMs: 0,
          resultPreview: '',
          state: 'running',
        },
      ],
    );

    expect(updated).toHaveLength(1);
    expect(updated[0].metadata?.tool_results).toEqual([
      {
        name: 'ShellExec',
        arguments: JSON.stringify({ command: 'cargo test' }),
        success: false,
        duration_ms: 0,
        result_preview: 'Cancelled before result',
      },
    ]);
  });

  it('treats only active streaming placeholders as live stream targets', () => {
    expect(isLiveStreamingAssistantMessage({ id: 'streaming-session-1' })).toBe(true);
    expect(isLiveStreamingAssistantMessage({ id: 'cancelled-run-1' })).toBe(false);
    expect(isLiveStreamingAssistantMessage({ id: 'error-run-1' })).toBe(false);
  });

  it('keeps local cancel and next-run placeholders when stale cancel reload completes', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      ...backendMessages,
      {
        id: 'cancelled-run-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{}',
              success: true,
              duration_ms: 20,
              result_preview: 'old output',
            },
          ],
        },
      },
      {
        id: 'user-continue',
        role: 'user',
        content: 'continue',
        timestamp: '2026-04-24T00:00:02.000Z',
        tool_calls: [],
      },
      {
        id: 'streaming-session-1',
        role: 'assistant',
        content: 'new output',
        timestamp: '2026-04-24T00:00:03.000Z',
        tool_calls: [],
        _streaming: true,
      } as Message,
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
      'user-continue',
      'streaming-session-1',
    ]);
  });

  it('keeps a cancelled snapshot before the continued turn after backend reload', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-user-2',
        role: 'user',
        content: 'continue',
        timestamp: '2026-04-24T00:00:02.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-2',
        role: 'assistant',
        content: 'continued answer',
        timestamp: '2026-04-24T00:00:04.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      backendMessages[0],
      {
        id: 'cancelled-run-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{}',
              success: true,
              duration_ms: 20,
              result_preview: 'old output',
            },
          ],
        },
      },
      {
        id: 'user-continue',
        role: 'user',
        content: 'continue',
        timestamp: '2026-04-24T00:00:02.000Z',
        tool_calls: [],
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
      'backend-user-2',
      'backend-assistant-2',
    ]);
  });
});
