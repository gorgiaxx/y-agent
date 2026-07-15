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
  const normalized = queue.map((item) => ({
    ...item,
    status: item.status ?? 'pending',
  }));
  if (normalized.length === 0) {
    if (!(sessionId in state)) return state;
    const next = { ...state };
    delete next[sessionId];
    return next;
  }
  return { ...state, [sessionId]: normalized };
}

/** Append an optimistic TODO without duplicating its authoritative echo. */
export function addTodo(
  state: TodoQueues,
  sessionId: string,
  item: TodoItem,
): TodoQueues {
  const existing = state[sessionId] ?? [];
  if (existing.some((candidate) => candidate.id === item.id)) return state;
  return {
    ...state,
    [sessionId]: [...existing, { ...item, status: item.status ?? 'pending' }],
  };
}

function setTodoStatus(
  state: TodoQueues,
  sessionId: string,
  todoId: string,
  status: TodoItem['status'],
): TodoQueues {
  const existing = state[sessionId];
  if (!existing) return state;
  let changed = false;
  const queue = existing.map((item) => {
    if (item.id !== todoId || item.status === status) return item;
    changed = true;
    return { ...item, status };
  });
  return changed ? { ...state, [sessionId]: queue } : state;
}

export function markTodoSteering(
  state: TodoQueues,
  sessionId: string,
  todoId: string,
): TodoQueues {
  return setTodoStatus(state, sessionId, todoId, 'steering');
}

export function markTodoPending(
  state: TodoQueues,
  sessionId: string,
  todoId: string,
): TodoQueues {
  return setTodoStatus(state, sessionId, todoId, 'pending');
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
