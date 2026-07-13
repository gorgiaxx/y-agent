import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { TodoQueue } from '../components/chat-panel/TodoQueue';
import type { TodoItem } from '../types';

function todo(id: string, text: string): TodoItem {
  return { id, text, created_at: 0 };
}

describe('TodoQueue', () => {
  it('renders pending items in numbered FIFO order', () => {
    const html = renderToStaticMarkup(
      <TodoQueue
        todos={[todo('a', 'run tests'), todo('b', 'write release notes')]}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    );

    expect(html).toContain('TODO (2)');
    expect(html.indexOf('run tests')).toBeLessThan(html.indexOf('write release notes'));
    expect(html).toContain('todo-queue-index">1');
    expect(html).toContain('todo-queue-index">2');
    expect(html).toContain('aria-label="Edit TODO item"');
    expect(html).toContain('aria-label="Delete TODO item"');
  });

  it('renders nothing when no TODO is pending', () => {
    expect(renderToStaticMarkup(
      <TodoQueue todos={[]} onEdit={() => {}} onDelete={() => {}} />,
    )).toBe('');
  });
});
