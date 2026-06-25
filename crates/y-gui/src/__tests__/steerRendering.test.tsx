import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { StaticBubble } from '../components/chat-panel/chat-box/StaticBubble';

describe('steer chip native rendering', () => {
  it('renders an inline steer chip between tool cards of a coalesced steered turn', () => {
    const html = renderToStaticMarkup(
      <StaticBubble
        message={{
          id: 'merged-1',
          role: 'assistant',
          content: 'look\nsearch\n',
          timestamp: '2026-06-25T00:00:00Z',
          tool_calls: [],
          metadata: {
            iteration_texts: ['look\n', 'search\n'],
            iteration_tool_counts: [1, 1],
            tool_results: [
              { name: 'Read', arguments: '{"file_path":"/x"}', success: true, duration_ms: 1, result_preview: 'a' },
              { name: 'Grep', arguments: '{"pattern":"y"}', success: true, duration_ms: 1, result_preview: 'b' },
            ],
            final_response: 'done',
            injected_steers: [
              { after_iteration: 1, text: 'focus on the parser', steer_id: 's1' },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('steer-chip');
    expect(html).toContain('Steered');
    expect(html).toContain('focus on the parser');
    // Both tool cards are still rendered.
    expect(html).toContain('Read');
    expect(html).toContain('Grep');

    // The steer chip sits between the first (Read) and second (Grep) tool card.
    const readIdx = html.indexOf('Read');
    const chipIdx = html.indexOf('steer-chip');
    const grepIdx = html.indexOf('Grep');
    expect(readIdx).toBeLessThan(chipIdx);
    expect(chipIdx).toBeLessThan(grepIdx);
  });
});
