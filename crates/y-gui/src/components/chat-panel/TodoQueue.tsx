import { ListTodo, Pencil, X } from 'lucide-react';

import type { TodoItem } from '../../types';
import './TodoQueue.css';

interface TodoQueueProps {
  todos: TodoItem[];
  onEdit: (todo: TodoItem) => void;
  onDelete: (todoId: string) => void;
}

/** Ordered deferred work for the currently streaming session. */
export function TodoQueue({ todos, onEdit, onDelete }: TodoQueueProps) {
  if (todos.length === 0) return null;

  return (
    <div className="todo-queue" role="list" aria-label="Pending TODO items">
      <div className="todo-queue-header">
        <ListTodo size={13} />
        <span>TODO ({todos.length})</span>
      </div>
      {todos.map((todo, index) => (
        <div className="todo-queue-item" role="listitem" key={todo.id}>
          <span className="todo-queue-index">{index + 1}</span>
          <span className="todo-queue-text" title={todo.text}>{todo.text}</span>
          <button
            type="button"
            className="todo-queue-action"
            onClick={() => onEdit(todo)}
            title="Edit TODO item"
            aria-label="Edit TODO item"
          >
            <Pencil size={13} />
          </button>
          <button
            type="button"
            className="todo-queue-action todo-queue-action--danger"
            onClick={() => onDelete(todo.id)}
            title="Delete TODO item"
            aria-label="Delete TODO item"
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
