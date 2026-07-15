import { describe, expect, it } from 'vitest';

import {
  addTodo,
  createTodoQueues,
  getTodoQueue,
  markTodoPending,
  markTodoSteering,
  removeTodo,
  setTodoQueue,
} from '../hooks/todoState';
import type { TodoItem } from '../types';

function todo(id: string, text = id): TodoItem {
  return { id, text, created_at: 0, status: 'pending' };
}

describe('todoState', () => {
  it('preserves FIFO order and deduplicates optimistic echoes', () => {
    let state = createTodoQueues();
    state = addTodo(state, 's1', todo('a'));
    state = addTodo(state, 's1', todo('b'));
    state = addTodo(state, 's1', todo('a'));

    expect(getTodoQueue(state, 's1').map((item) => item.id)).toEqual(['a', 'b']);
  });

  it('removes injected items and clears terminal queues', () => {
    let state = createTodoQueues();
    state = setTodoQueue(state, 's1', [todo('a'), todo('b')]);
    state = removeTodo(state, 's1', 'a');
    expect(getTodoQueue(state, 's1').map((item) => item.id)).toEqual(['b']);

    state = setTodoQueue(state, 's1', []);
    expect(getTodoQueue(state, 's1')).toEqual([]);
    expect('s1' in state).toBe(false);
  });

  it('keeps a steered TODO visible until injection and allows reverting it', () => {
    let state = setTodoQueue(createTodoQueues(), 's1', [todo('a'), todo('b')]);

    state = markTodoSteering(state, 's1', 'a');
    expect(getTodoQueue(state, 's1')[0].status).toBe('steering');

    state = markTodoPending(state, 's1', 'a');
    expect(getTodoQueue(state, 's1')[0].status).toBe('pending');

    state = markTodoSteering(state, 's1', 'a');
    state = removeTodo(state, 's1', 'a');
    expect(getTodoQueue(state, 's1').map((item) => item.id)).toEqual(['b']);
  });
});
