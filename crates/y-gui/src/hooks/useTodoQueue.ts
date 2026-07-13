import { useCallback, useEffect, useMemo, useState } from 'react';

import { logger, transport } from '../lib';
import type { TodoItem } from '../types';
import { chatBusSubscribers, type ChatBusEvent } from './chatBus';
import {
  addTodo as addToQueue,
  createTodoQueues,
  getTodoQueue,
  removeTodo as removeFromQueue,
  setTodoQueue,
  type TodoQueues,
} from './todoState';

export interface UseTodoQueueReturn {
  todosFor: (sessionId: string | null) => TodoItem[];
  addTodo: (sessionId: string, text: string) => Promise<void>;
  deleteTodo: (sessionId: string, todoId: string) => Promise<void>;
}

/** Reactive projection of the service-owned per-session TODO queues. */
export function useTodoQueue(): UseTodoQueueReturn {
  const [queues, setQueues] = useState<TodoQueues>(createTodoQueues);

  useEffect(() => {
    const handler = (event: ChatBusEvent) => {
      if (event.type === 'todo_queue') {
        setQueues((prev) => setTodoQueue(prev, event.session_id, event.queue));
      } else if (event.type === 'todo_injected') {
        setQueues((prev) => removeFromQueue(prev, event.session_id, event.todo_id));
      } else if (event.type === 'complete' || event.type === 'error') {
        setQueues((prev) => setTodoQueue(prev, event.payload.session_id, []));
      }
    };
    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  }, []);

  const todosFor = useCallback(
    (sessionId: string | null) => (sessionId ? getTodoQueue(queues, sessionId) : []),
    [queues],
  );

  const addTodo = useCallback(async (sessionId: string, text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    try {
      const item = await transport.invoke<TodoItem>('chat_add_follow_up', {
        sessionId,
        text: trimmed,
      });
      setQueues((prev) => addToQueue(prev, sessionId, item));
    } catch (error) {
      logger.error('[useTodoQueue] add TODO failed:', error);
      throw error;
    }
  }, []);

  const deleteTodo = useCallback(async (sessionId: string, todoId: string) => {
    setQueues((prev) => removeFromQueue(prev, sessionId, todoId));
    try {
      await transport.invoke('chat_delete_follow_up', { sessionId, followUpId: todoId });
    } catch (error) {
      logger.error('[useTodoQueue] delete TODO failed:', error);
      throw error;
    }
  }, []);

  return useMemo(
    () => ({ todosFor, addTodo, deleteTodo }),
    [todosFor, addTodo, deleteTodo],
  );
}
