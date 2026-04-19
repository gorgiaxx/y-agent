import { renderToStaticMarkup } from 'react-dom/server';
import { beforeAll, describe, expect, it, vi } from 'vitest';

import type { Message } from '../types';

class MockEventSource {
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  addEventListener() {}
  close() {}
}

beforeAll(() => {
  vi.stubGlobal('EventSource', MockEventSource);
});

describe('generated image rendering', () => {
  it('renders persisted assistant generated images from message metadata', async () => {
    const { StaticBubble } = await import('../components/chat-panel/chat-box/StaticBubble');

    const message: Message = {
      id: 'assistant-1',
      role: 'assistant',
      content: 'Here is your image.',
      timestamp: new Date('2026-04-19T09:00:00Z').toISOString(),
      tool_calls: [],
      metadata: {
        generated_images: [
          {
            index: 0,
            mime_type: 'image/png',
            data: 'iVBORw0KGgo=',
          },
        ],
      },
    };

    const html = renderToStaticMarkup(<StaticBubble message={message} />);

    expect(html).toContain('data:image/png;base64,iVBORw0KGgo=');
  });
});
