import { ListTodo, Navigation, Pencil, RotateCcw, X } from 'lucide-react';

import type { TodoItem } from '../../types';
import './TodoQueue.css';

interface TodoQueueProps {
  todos: TodoItem[];
  onSteer: (todo: TodoItem) => void;
  onUndoSteer: (todo: TodoItem) => void;
  onEdit: (todo: TodoItem) => void;
  onDelete: (todoId: string) => void;
}

/** Ordered deferred work for the currently streaming session. */
export function TodoQueue({ todos, onSteer, onUndoSteer, onEdit, onDelete }: TodoQueueProps) {
  if (todos.length === 0) return null;
  const hasSteeringTodo = todos.some((todo) => todo.status === 'steering');

  return (
    <div className="todo-queue" role="list" aria-label="TODO items">
      <div className="todo-queue-header">
        <ListTodo size={13} />
        <span>TODO ({todos.length})</span>
      </div>
      {todos.map((todo, index) => {
        const isSteering = todo.status === 'steering';
        return (
          <div
            className={`todo-queue-item${isSteering ? ' todo-queue-item--steering' : ''}`}
            role="listitem"
            key={todo.id}
          >
            <span className="todo-queue-index">{index + 1}</span>
            <span className="todo-queue-text" title={todo.text}>{todo.text}</span>
            {isSteering ? (
              <>
                <span className="todo-queue-status">Steering</span>
                <button
                  type="button"
                  className="todo-queue-action"
                  onClick={() => onUndoSteer(todo)}
                  title="Undo steer"
                  aria-label="Undo steer TODO item"
                >
                  <RotateCcw size={13} />
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="todo-queue-action"
                  onClick={() => onSteer(todo)}
                  title="Steer with this TODO at the next step"
                  aria-label="Steer TODO item"
                  disabled={hasSteeringTodo}
                >
                  <Navigation size={13} />
                </button>
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
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}
