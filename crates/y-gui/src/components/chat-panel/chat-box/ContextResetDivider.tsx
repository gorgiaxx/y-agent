import { RotateCcw, Scissors } from 'lucide-react';
import './ContextResetDivider.css';

interface ContextResetDividerProps {
  onUndo?: () => void;
}

export function ContextResetDivider({ onUndo }: ContextResetDividerProps) {
  return (
    <div className="context-reset-divider" role="separator">
      <span className="context-reset-divider-line" />
      <span className="context-reset-divider-label">
        <Scissors size={12} />
        <span>Context reset -- messages above are not sent to the model</span>
      </span>
      {onUndo && (
        <button
          type="button"
          className="context-reset-divider-undo"
          onClick={onUndo}
          title="Undo context reset and use the earlier messages again"
          aria-label="Undo context reset"
        >
          <RotateCcw size={12} />
          <span>Undo</span>
        </button>
      )}
      <span className="context-reset-divider-line" />
    </div>
  );
}
