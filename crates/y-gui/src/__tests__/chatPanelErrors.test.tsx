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

describe('ChatPanel intra-turn failure', () => {
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
});
