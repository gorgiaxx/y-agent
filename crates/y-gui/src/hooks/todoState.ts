import type { TodoItem } from '../types';

/** Per-session TODO queues, keyed by session id. */
export type TodoQueues = Record<string, TodoItem[]>;

export function createTodoQueues(): TodoQueues {
  return {};
}

export function getTodoQueue(state: TodoQueues, sessionId: string): TodoItem[] {
  return state[sessionId] ?? [];
}

/** Replace a session's queue with the authoritative server order. */
export function setTodoQueue(
  state: TodoQueues,
  sessionId: string,
  queue: TodoItem[],
): TodoQueues {
  if (queue.length === 0) {
    if (!(sessionId in state)) return state;
    const next = { ...state };
    delete next[sessionId];
    return next;
  }
  return { ...state, [sessionId]: queue };
}

/** Append an optimistic TODO without duplicating its authoritative echo. */
export function addTodo(
  state: TodoQueues,
  sessionId: string,
  item: TodoItem,
): TodoQueues {
  const existing = state[sessionId] ?? [];
  if (existing.some((candidate) => candidate.id === item.id)) return state;
  return { ...state, [sessionId]: [...existing, item] };
}

export function removeTodo(
  state: TodoQueues,
  sessionId: string,
  todoId: string,
): TodoQueues {
  const existing = state[sessionId];
  if (!existing) return state;
  const filtered = existing.filter((item) => item.id !== todoId);
  if (filtered.length === existing.length) return state;
  return setTodoQueue(state, sessionId, filtered);
}
