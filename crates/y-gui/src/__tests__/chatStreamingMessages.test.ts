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

  it('preserves provider errors as visible terminal assistant messages', () => {
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
      'error-run-1',
      undefined,
      'LLM error: server error from DeepSeek-V4: HTTP 400 Bad Request: read timeout',
    );

    expect(updated).toHaveLength(1);
    expect(updated[0]).toMatchObject({
      id: 'error-run-1',
      role: 'assistant',
      metadata: {
        stream_error: 'LLM error: server error from DeepSeek-V4: HTTP 400 Bad Request: read timeout',
      },
    });
    expect(updated[0]).not.toHaveProperty('_streaming');
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

  it('deduplicates when backend returns the persisted assistant for a cancelled turn', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      backendMessages[0],
      {
        id: 'cancelled-run-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{}',
              success: true,
              duration_ms: 20,
              result_preview: 'some output',
            },
          ],
        },
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
    ]);
    expect(merged[1].metadata?.tool_results).toBeDefined();
  });

  it('deduplicates after session switch with optimistic user message', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      {
        id: 'user-1714003200000',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'cancelled-run-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{}',
              success: true,
              duration_ms: 20,
              result_preview: 'some output',
            },
          ],
        },
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
    ]);
    expect(merged[1].metadata?.tool_results).toBeDefined();
  });

  it('deduplicates through cancel reload then session switch reload', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
      },
    ];
    const cancelCached: Message[] = [
      {
        id: 'user-1714003200000',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'cancelled-run-1',
        role: 'assistant',
        content: 'I will start by reading',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{}',
              success: true,
              duration_ms: 20,
              result_preview: 'some output',
            },
          ],
        },
      },
    ];

    // Step B: cancel handler async reload
    const afterCancelReload = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cancelCached,
    );
    expect(afterCancelReload.map((m) => m.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
    ]);

    // Session switch: loadMessages reload using post-cancel cache
    const afterSessionSwitch = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      afterCancelReload,
    );
    expect(afterSessionSwitch.map((m) => m.id)).toEqual([
      'backend-user-1',
      'cancelled-run-1',
    ]);
  });

  it('deduplicates in multi-turn conversation with prior assistant messages', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'First message',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: 'First reply',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-user-2',
        role: 'user',
        content: 'Second message',
        timestamp: '2026-04-24T00:00:02.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-2',
        role: 'assistant',
        content: 'Partial second reply',
        timestamp: '2026-04-24T00:00:03.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      backendMessages[0],
      backendMessages[1],
      {
        id: 'user-1714003202000',
        role: 'user',
        content: 'Second message',
        timestamp: '2026-04-24T00:00:02.000Z',
        tool_calls: [],
      },
      {
        id: 'cancelled-run-2',
        role: 'assistant',
        content: 'Partial second reply',
        timestamp: '2026-04-24T00:00:03.000Z',
        tool_calls: [],
        metadata: {
          tool_results: [
            {
              name: 'ShellExec',
              arguments: '{}',
              success: true,
              duration_ms: 100,
              result_preview: 'output',
            },
          ],
        },
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((m) => m.id)).toEqual([
      'backend-user-1',
      'backend-assistant-1',
      'backend-user-2',
      'cancelled-run-2',
    ]);
    expect(merged[3].metadata?.tool_results).toBeDefined();
  });

  it('deduplicates when backend returns the persisted assistant for an error turn', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Start the task',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
      },
    ];
    const cachedMessages: Message[] = [
      backendMessages[0],
      {
        id: 'error-run-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          stream_error: 'LLM error: timeout',
        },
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'error-run-1',
    ]);
    expect(merged[1].metadata?.stream_error).toBe('LLM error: timeout');
  });

  it('prefers the backend-renderable assistant when an LLM error persisted tool metadata', () => {
    const backendMessages: Message[] = [
      {
        id: 'backend-user-1',
        role: 'user',
        content: 'Read a missing file',
        timestamp: '2026-04-24T00:00:00.000Z',
        tool_calls: [],
      },
      {
        id: 'backend-assistant-1',
        role: 'assistant',
        content: 'I will inspect that file.\n',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          stream_error: 'LLM error: provider rejected tool result',
          iteration_texts: ['I will inspect that file.\n'],
          iteration_tool_counts: [1],
          tool_results: [
            {
              name: 'FileRead',
              arguments: '{"path":"/missing.rs"}',
              success: false,
              duration_ms: 0,
              result_preview: '{"error":"file not found"}',
            },
          ],
        },
      },
    ];
    const cachedMessages: Message[] = [
      backendMessages[0],
      {
        id: 'error-run-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          stream_error: 'LLM error: provider rejected tool result',
        },
      },
    ];

    const merged = mergeBackendMessagesPreservingLocalStreamState(
      backendMessages,
      cachedMessages,
    );

    expect(merged.map((message) => message.id)).toEqual([
      'backend-user-1',
      'backend-assistant-1',
    ]);
    expect(merged[1].metadata?.tool_results).toEqual(
      backendMessages[1].metadata?.tool_results,
    );
    expect(merged[1].content).toBe('I will inspect that file.\n');
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
