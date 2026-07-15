import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { StaticBubble } from '../components/chat-panel/chat-box/StaticBubble';
import type { Message } from '../types';

function message(metadata?: Record<string, unknown>): Message {
  return {
    id: 'assistant-1',
    role: 'assistant',
    content: 'Completed response',
    timestamp: '2026-07-15T00:00:00Z',
    tool_calls: [],
    metadata,
  };
}

describe('assistant feedback rendering', () => {
  it('shows evolution feedback controls only when a diagnostics trace is available', () => {
    const traced = renderToStaticMarkup(
      <StaticBubble message={message({ trace_id: '11111111-1111-4111-8111-111111111111' })} />,
    );
    const untraced = renderToStaticMarkup(<StaticBubble message={message()} />);

    expect(traced).toContain('Good response');
    expect(traced).toContain('Bad response');
    expect(untraced).not.toContain('Good response');
    expect(untraced).not.toContain('Bad response');
  });
});
