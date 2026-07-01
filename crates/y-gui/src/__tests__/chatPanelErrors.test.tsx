import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ChatPanel } from '../components/chat-panel/ChatPanel';
import type { Message } from '../types';

const PROVIDER_ERROR =
  'LLM error: server error from DeepSeek-V4: HTTP 504 Gateway Timeout: backend request timeout';

function userMessage(id: string, content: string): Message {
  return {
    id,
    role: 'user',
    content,
    timestamp: '2026-04-24T00:00:00.000Z',
    tool_calls: [],
  };
}

function erroredAssistant(id: string): Message {
  return {
    id,
    role: 'assistant',
    content: '',
    timestamp: '2026-04-24T00:00:01.000Z',
    tool_calls: [],
    metadata: { stream_error: PROVIDER_ERROR },
  };
}

// Mirrors the standalone "successful iteration" message the backend persists
// before the failure marker: completed work (content + tool_results), no
// stream_error.
function successWithToolCall(id: string): Message {
  return {
    id,
    role: 'assistant',
    content: 'I will inspect the file.',
    timestamp: '2026-04-24T00:00:01.000Z',
    tool_calls: [],
    metadata: {
      iteration_texts: ['I will inspect the file.'],
      tool_results: [
        {
          name: 'do_work',
          arguments: '{}',
          success: true,
          duration_ms: 5,
          result_preview: 'ok',
        },
      ],
    },
  };
}

describe('ChatPanel error rendering', () => {
  it('does not render a duplicate global error when the assistant bubble already shows the stream error', () => {
    const messages: Message[] = [erroredAssistant('error-run-1')];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={PROVIDER_ERROR}
      />,
    );

    expect(html).toContain('Provider error');
    expect(html).not.toContain('chat-error');
  });
});

describe('ChatPanel retry affordance', () => {
  it('renders a Retry button on the provider-error bubble when a preceding user turn can be retried', () => {
    const messages: Message[] = [
      userMessage('user-1', 'summarize this'),
      erroredAssistant('error-run-1'),
    ];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={PROVIDER_ERROR}
        onRetryTurn={() => {}}
      />,
    );

    expect(html).toContain('Provider error');
    expect(html).toContain('assistant-error-retry-btn');
  });

  it('omits the Retry button when no retry handler is wired', () => {
    const messages: Message[] = [
      userMessage('user-1', 'summarize this'),
      erroredAssistant('error-run-1'),
    ];

    const html = renderToStaticMarkup(
      <ChatPanel messages={messages} isStreaming={false} isLoading={false} error={null} />,
    );

    expect(html).toContain('Provider error');
    expect(html).not.toContain('assistant-error-retry-btn');
  });

  it('omits the Retry button when there is no preceding user message to retry', () => {
    const messages: Message[] = [erroredAssistant('error-run-1')];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={null}
        onRetryTurn={() => {}}
      />,
    );

    expect(html).toContain('Provider error');
    expect(html).not.toContain('assistant-error-retry-btn');
  });

  it('renders a Retry button on the global error banner targeting the last user turn', () => {
    const messages: Message[] = [userMessage('user-1', 'do the thing')];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={PROVIDER_ERROR}
        onRetryTurn={() => {}}
      />,
    );

    expect(html).toContain('chat-error');
    expect(html).toContain('chat-error-retry-btn');
  });
});

describe('ChatPanel load failure', () => {
  it('surfaces a load error instead of the welcome screen when there are no messages', () => {
    // Bug B: a failed session_get_messages (e.g. corrupt transcript) left the
    // panel showing the "Welcome" empty state, hiding the failure entirely.
    const html = renderToStaticMarkup(
      <ChatPanel
        messages={[]}
        isStreaming={false}
        isLoading={false}
        error={'Failed to read display transcript: parse message'}
      />,
    );

    expect(html).not.toContain('Welcome to y-agent');
    expect(html).toContain('chat-error');
    expect(html).toContain('Failed to read display transcript');
  });

  it('still shows the welcome screen for a genuinely empty session with no error', () => {
    const html = renderToStaticMarkup(
      <ChatPanel messages={[]} isStreaming={false} isLoading={false} error={null} />,
    );

    expect(html).toContain('Welcome to y-agent');
  });
});

describe('ChatPanel intra-turn failure', () => {
  it('keeps the streamed text visible on a single error bubble that also ran a tool', () => {
    // Bug A: when the error label appeared, the streaming message was re-id'd
    // to error-* and rendered via history segments built from iteration_texts.
    // finalizeStreamingAssistantMessage now projects the live segments into
    // that metadata, so text (and reasoning) render alongside the tool card
    // instead of being dropped.
    const erroredWithWork: Message = {
      id: 'error-run-1',
      role: 'assistant',
      content: 'Let me inspect the file.',
      timestamp: '2026-04-24T00:00:01.000Z',
      tool_calls: [],
      metadata: {
        stream_error: PROVIDER_ERROR,
        iteration_texts: ['Let me inspect the file.'],
        iteration_reasonings: [null],
        iteration_tool_counts: [1],
        tool_results: [
          { name: 'do_work', arguments: '{}', success: true, duration_ms: 5, result_preview: 'ok' },
        ],
      },
    };

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={[userMessage('user-1', 'inspect the file'), erroredWithWork]}
        isStreaming={false}
        isLoading={false}
        error={PROVIDER_ERROR}
      />,
    );

    // Streamed text survives the switch to the static/error bubble.
    expect(html).toContain('Let me inspect the file.');
    // Error notice still renders.
    expect(html).toContain('Provider error');
  });

  it('keeps the completed tool-call bubble visible alongside the failure marker', () => {
    // A turn that ran a tool and then hit a later LLM failure is persisted as
    // TWO display messages: the completed work (no stream_error) followed by a
    // lightweight failure marker. Both must render -- the work stays visible
    // and is NOT wiped, and the marker carries the error + Retry affordance.
    const messages: Message[] = [
      userMessage('user-1', 'inspect the file'),
      successWithToolCall('assistant-work-1'),
      erroredAssistant('error-run-1'),
    ];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={PROVIDER_ERROR}
        onRetryTurn={() => {}}
      />,
    );

    // Completed work remains visible (the bug was that retry wiped it).
    expect(html).toContain('I will inspect the file.');
    // Failure marker still renders its error notice.
    expect(html).toContain('Provider error');
    // Retry stays available -- findPrecedingUserMessage skips the success
    // bubble in between and targets the original user turn.
    expect(html).toContain('assistant-error-retry-btn');
  });
  it('renders the failure marker with a retry button when a retry also fails with no new work', () => {
    // Regression: when the user retries with a different model and that
    // attempt also fails immediately (e.g. rate-limited before any content),
    // the backend now persists a bare failure marker (empty content, only
    // stream_error). The marker must render its error notice + Retry button
    // so the user can try again -- previously the marker was never persisted
    // and the entire turn's work was wiped to just a global error banner.
    const RATE_LIMIT_ERROR = 'LLM error: rate limited by SenseNova: retry after 60s';
    const rateLimitedMarker: Message = {
      id: 'error-run-2',
      role: 'assistant',
      content: '',
      timestamp: '2026-04-24T00:00:02.000Z',
      tool_calls: [],
      metadata: { stream_error: RATE_LIMIT_ERROR },
    };
    const messages: Message[] = [
      userMessage('user-1', 'write a function'),
      successWithToolCall('assistant-work-1'),
      rateLimitedMarker,
    ];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={RATE_LIMIT_ERROR}
        onRetryTurn={() => {}}
      />,
    );

    // The completed work from the first attempt stays visible.
    expect(html).toContain('I will inspect the file.');
    // The failure marker renders the rate-limit error.
    expect(html).toContain('Provider error');
    expect(html).toContain('rate limited by SenseNova');
    // Retry is available on the failure marker.
    expect(html).toContain('assistant-error-retry-btn');
    // No duplicate global error banner (the marker already shows it).
    expect(html).not.toContain('chat-error');
  });
});
