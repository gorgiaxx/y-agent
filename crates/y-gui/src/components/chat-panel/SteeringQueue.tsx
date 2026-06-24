import { Pencil, X, Navigation } from 'lucide-react';
import type { SteerMessage } from '../../types';
import './SteeringQueue.css';

interface SteeringQueueProps {
  steers: SteerMessage[];
  /** Edit: remove from queue and repopulate the input box with its text. */
  onEdit: (steer: SteerMessage) => void;
  /** Delete: remove from queue. */
  onDelete: (steerId: string) => void;
}

/**
 * Pending steering messages shown above the input area while a run streams.
 * Each item is injected into the agent at the next LLM-call boundary; until
 * then the user can edit (pull back into the input) or delete it.
 */
export function SteeringQueue({ steers, onEdit, onDelete }: SteeringQueueProps) {
  if (steers.length === 0) return null;

  return (
    <div className="steering-queue" role="list" aria-label="Pending steering messages">
      <div className="steering-queue-header">
        <Navigation size={12} />
        <span>Steering ({steers.length}) -- applied at next step</span>
      </div>
      {steers.map((steer) => (
        <div className="steering-queue-item" role="listitem" key={steer.id}>
          <span className="steering-queue-text" title={steer.text}>{steer.text}</span>
          <button
            type="button"
            className="steering-queue-action"
            onClick={() => onEdit(steer)}
            title="Edit (move back to input)"
            aria-label="Edit steering message"
          >
            <Pencil size={13} />
          </button>
          <button
            type="button"
            className="steering-queue-action steering-queue-action--danger"
            onClick={() => onDelete(steer.id)}
            title="Remove from queue"
            aria-label="Delete steering message"
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
