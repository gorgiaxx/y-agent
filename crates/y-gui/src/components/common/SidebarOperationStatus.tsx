import { useState } from 'react';
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock3,
  Copy,
  X,
} from 'lucide-react';

export type SidebarOperationState = 'idle' | 'running' | 'success' | 'error';

interface SidebarOperationStatusProps {
  status: SidebarOperationState;
  runningMessage: string;
  successMessage: string;
  errorMessage?: string | null;
  fallbackErrorMessage?: string;
  onDismiss: () => void;
  onCancel?: () => void;
}

export function SidebarOperationStatus({
  status,
  runningMessage,
  successMessage,
  errorMessage,
  fallbackErrorMessage = 'Operation failed',
  onDismiss,
  onCancel,
}: SidebarOperationStatusProps) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);

  if (status === 'idle') return null;

  const message = status === 'running'
    ? runningMessage
    : status === 'success'
      ? successMessage
      : errorMessage || fallbackErrorMessage;

  const dismiss = () => {
    setExpanded(false);
    onDismiss();
  };

  const copyError = () => {
    if (status !== 'error') return;
    void navigator.clipboard.writeText(message).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className={`import-status import-status--${status === 'running' ? 'importing' : status} ${expanded ? 'import-status--expanded' : ''}`}>
      <div className="import-status-row">
        {status === 'running' && <Clock3 size={14} className="import-status-busy-icon" />}
        {status === 'success' && <CheckCircle2 size={14} />}
        {status === 'error' && <AlertCircle size={14} className="import-status-icon" />}
        <span className={`import-status-msg ${expanded ? 'import-status-msg--expanded' : ''}`}>
          {message}
        </span>
        <div className="import-status-actions">
          {status === 'error' && (
            <button
              type="button"
              className={`import-status-copy ${copied ? 'import-status-copy--copied' : ''}`}
              onClick={copyError}
              title="Copy error"
              aria-label="Copy error"
            >
              {copied ? <Check size={12} /> : <Copy size={12} />}
            </button>
          )}
          {status === 'error' && (
            <button
              type="button"
              className="import-status-toggle"
              onClick={() => setExpanded((current) => !current)}
              title={expanded ? 'Collapse' : 'Expand'}
              aria-label={expanded ? 'Collapse error' : 'Expand error'}
              aria-expanded={expanded}
            >
              {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            </button>
          )}
          {status === 'running' && onCancel ? (
            <button
              type="button"
              className="import-status-dismiss"
              onClick={onCancel}
              title="Cancel"
              aria-label="Cancel"
            >
              <X size={12} />
            </button>
          ) : status !== 'running' ? (
            <button
              type="button"
              className="import-status-dismiss"
              onClick={dismiss}
              title="Dismiss"
              aria-label="Dismiss"
            >
              <X size={12} />
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
