import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { SteeringQueue } from '../components/chat-panel/SteeringQueue';
import type { SteerMessage } from '../types';

function steer(id: string, text: string): SteerMessage {
  return { id, text, created_at: 0 };
}

describe('SteeringQueue', () => {
  it('renders nothing when the queue is empty', () => {
    const html = renderToStaticMarkup(
      <SteeringQueue steers={[]} onEdit={() => {}} onDelete={() => {}} />,
    );
    expect(html).toBe('');
  });

  it('renders each pending steer with edit and delete actions', () => {
    const steers = [steer('a', 'focus on tests'), steer('b', 'use the new API')];
    const html = renderToStaticMarkup(
      <SteeringQueue steers={steers} onEdit={() => {}} onDelete={() => {}} />,
    );

    expect(html).toContain('focus on tests');
    expect(html).toContain('use the new API');
    expect(html).toContain('Steering (2)');
    expect(html).toContain('aria-label="Edit steering message"');
    expect(html).toContain('aria-label="Delete steering message"');
  });
});
