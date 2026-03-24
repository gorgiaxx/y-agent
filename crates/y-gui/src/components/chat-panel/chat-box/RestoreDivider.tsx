import { RotateCcw } from 'lucide-react';
import './RestoreDivider.css';

interface RestoreDividerProps {
  checkpointId: string;
  tombstonedCount: number;
  onRestore: (checkpointId: string) => void;
}

export function RestoreDivider({ checkpointId, tombstonedCount, onRestore }: RestoreDividerProps) {
  return (
    <div className="restore-divider" role="separator">
      <span className="restore-divider-line" />
      <button
        className="restore-divider-label"
        onClick={() => onRestore(checkpointId)}
        onKeyDown={(e) => { if (e.key === 'Enter') onRestore(checkpointId); }}
        tabIndex={0}
        role="button"
        aria-label={`Restore ${tombstonedCount} removed message${tombstonedCount !== 1 ? 's' : ''}`}
      >
        <RotateCcw size={12} />
        <span>Restore {tombstonedCount} message{tombstonedCount !== 1 ? 's' : ''}</span>
      </button>
      <span className="restore-divider-line" />
    </div>
  );
}
