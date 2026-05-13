import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ChatPanel } from '../components/chat-panel/ChatPanel';
import type { Message } from '../types';

describe('ChatPanel error rendering', () => {
  it('does not render a duplicate global error when the assistant bubble already shows the stream error', () => {
    const providerError = 'LLM error: server error from DeepSeek-V4: HTTP 400 Bad Request: read timeout';
    const messages: Message[] = [
      {
        id: 'error-run-1',
        role: 'assistant',
        content: '',
        timestamp: '2026-04-24T00:00:01.000Z',
        tool_calls: [],
        metadata: {
          stream_error: providerError,
        },
      },
    ];

    const html = renderToStaticMarkup(
      <ChatPanel
        messages={messages}
        isStreaming={false}
        isLoading={false}
        error={providerError}
      />,
    );

    expect(html).toContain('Provider error');
    expect(html).not.toContain('chat-error');
  });
});
