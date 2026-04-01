import { useEffect, useRef } from 'react';
import { ShieldAlert, Check, X, Terminal, FileText, Globe } from 'lucide-react';
import './PermissionDialog.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PermissionDialogProps {
  /** Unique request ID from the backend. */
  requestId: string;
  /** Tool name requesting permission. */
  toolName: string;
  /** Human-readable description of the action. */
  actionDescription: string;
  /** Why permission is required. */
  reason: string;
  /** Optional content preview (command text, file path, etc.). */
  contentPreview?: string | null;
  /** Callback when user approves. */
  onApprove: (requestId: string) => void;
  /** Callback when user denies. */
  onDeny: (requestId: string) => void;
  /** Callback when user allows all future tool calls for this session. */
  onAllowAllForSession: (requestId: string) => void;
}

// ---------------------------------------------------------------------------
// Helper: pick an icon based on tool name
// ---------------------------------------------------------------------------

function toolIcon(toolName: string) {
  const name = toolName.toLowerCase();
  if (name.includes('shell') || name.includes('exec') || name.includes('bash')) {
    return <Terminal size={16} className="permission-tool-icon" />;
  }
  if (name.includes('file') || name.includes('write') || name.includes('read')) {
    return <FileText size={16} className="permission-tool-icon" />;
  }
  if (name.includes('http') || name.includes('url') || name.includes('fetch') || name.includes('browser')) {
    return <Globe size={16} className="permission-tool-icon" />;
  }
  return <ShieldAlert size={16} className="permission-tool-icon" />;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * PermissionDialog -- floating panel above InputArea for tool permission approval.
 *
 * Rendered when the backend's permission gatekeeper determines a tool
 * requires user approval (default_permission = "ask" or dangerous_auto_ask).
 *
 * Shows the tool name, action description, and optional content preview.
 * User can Allow once, deny, or allow all future tool calls for the session.
 * Keyboard: Enter = Allow, Shift+Enter = Allow All for Session, Escape = Deny.
 */
export function PermissionDialog({
  requestId,
  toolName,
  actionDescription,
  reason,
  contentPreview,
  onApprove,
  onDeny,
  onAllowAllForSession,
}: PermissionDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);

  // Keyboard shortcuts: Enter = Allow, Shift+Enter = Allow All, Escape = Deny.
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        onAllowAllForSession(requestId);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        onApprove(requestId);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        onDeny(requestId);
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [requestId, onApprove, onDeny, onAllowAllForSession]);

  // Auto-focus on mount.
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  return (
    <div className="permission-dialog" ref={dialogRef} tabIndex={-1}>
      {/* Header */}
      <div className="permission-header">
        <ShieldAlert size={16} className="permission-header-icon" />
        <span className="permission-header-title">Permission Required</span>
      </div>

      {/* Body */}
      <div className="permission-body">
        <div className="permission-tool-row">
          {toolIcon(toolName)}
          <span className="permission-tool-name">{toolName}</span>
        </div>
        <div className="permission-description">{actionDescription}</div>
        {contentPreview && (
          <div className="permission-preview">
            <code className="permission-preview-code">{contentPreview}</code>
          </div>
        )}
        <div className="permission-reason">{reason}</div>
      </div>

      {/* Footer */}
      <div className="permission-footer">
        <span className="permission-hint">Enter = Allow, Shift+Enter = Allow All, Esc = Deny</span>
        <div className="permission-actions">
          <button
            className="permission-btn permission-btn--deny"
            onClick={() => onDeny(requestId)}
          >
            <X size={14} />
            Deny
          </button>
          <button
            className="permission-btn permission-btn--allow"
            onClick={() => onApprove(requestId)}
          >
            <Check size={14} />
            Allow
          </button>
          <button
            className="permission-btn permission-btn--allow-all"
            onClick={() => onAllowAllForSession(requestId)}
          >
            <Check size={14} />
            Allow All for Session
          </button>
        </div>
      </div>
    </div>
  );
}
