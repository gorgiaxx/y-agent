import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { TodoQueue } from '../components/chat-panel/TodoQueue';
import type { TodoItem } from '../types';

function todo(id: string, text: string): TodoItem {
  return { id, text, created_at: 0, status: 'pending' };
}

describe('TodoQueue', () => {
  it('renders pending items in numbered FIFO order', () => {
    const html = renderToStaticMarkup(
      <TodoQueue
        todos={[todo('a', 'run tests'), todo('b', 'write release notes')]}
        onSteer={() => {}}
        onUndoSteer={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    );

    expect(html).toContain('TODO (2)');
    expect(html.indexOf('run tests')).toBeLessThan(html.indexOf('write release notes'));
    expect(html).toContain('todo-queue-index">1');
    expect(html).toContain('todo-queue-index">2');
    expect(html).toContain('aria-label="Steer TODO item"');
    expect(html).toContain('aria-label="Edit TODO item"');
    expect(html).toContain('aria-label="Delete TODO item"');
  });

  it('renders a steering status with an undo action until injection', () => {
    const steering = { ...todo('a', 'redirect the run'), status: 'steering' as const };
    const html = renderToStaticMarkup(
      <TodoQueue
        todos={[steering, todo('b', 'keep queued')]}
        onSteer={() => {}}
        onUndoSteer={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    );

    expect(html).toContain('Steering');
    expect(html).toContain('aria-label="Undo steer TODO item"');
    expect(html.match(/aria-label="Edit TODO item"/g)).toHaveLength(1);
    expect(html).toContain('aria-label="Steer TODO item" disabled=""');
  });

  it('renders nothing when no TODO is pending', () => {
    expect(renderToStaticMarkup(
      <TodoQueue
        todos={[]}
        onSteer={() => {}}
        onUndoSteer={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    )).toBe('');
  });
});
