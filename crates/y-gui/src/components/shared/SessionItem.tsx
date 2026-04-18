import type { ReactNode } from 'react';
import type { SessionInfo } from '../../types';
import { Pin } from 'lucide-react';
import { formatSessionRelativeTime } from '../chat-panel/sessionListActivity';
import './SessionItem.css';

export interface SessionItemProps {
  session: SessionInfo;
  isActive: boolean;
  isStreaming: boolean;
  isPinned?: boolean;
  subtitle?: string;
  actions?: ReactNode;
  className?: string;
  onClick: (e: React.MouseEvent) => void;
  onPinToggle?: (e: React.MouseEvent) => void;
  onMouseDown?: (e: React.MouseEvent) => void;
  onMouseMove?: (e: React.MouseEvent) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}

export function SessionItem({
  session,
  isActive,
  isStreaming,
  isPinned,
  subtitle,
  actions,
  className = '',
  onClick,
  onPinToggle,
  onMouseDown,
  onMouseMove,
  onContextMenu,
}: SessionItemProps) {
  const timeLabel = formatSessionRelativeTime(session.updated_at, isStreaming);

  return (
    <div
      className={`session-item ${isActive ? 'session-item--active' : ''} ${isStreaming ? 'session-item--streaming' : ''} ${className}`}
      onClick={onClick}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onContextMenu={onContextMenu}
    >
      {isStreaming ? (
        <span className="session-item-spinner" aria-hidden="true" />
      ) : isPinned || onPinToggle ? (
        <button
          className={`session-item-pin${isPinned ? ' session-item-pin--pinned' : ''}`}
          onClick={(e) => {
            e.stopPropagation();
            onPinToggle?.(e);
          }}
          aria-label={isPinned ? 'Unpin session' : 'Pin session'}
        >
          <Pin size={12} />
        </button>
      ) : (
        <span className="session-item-spinner-placeholder" aria-hidden="true" />
      )}

      <div className="session-item-content">
        <div className="session-item-title">
          {session.title || 'Untitled'}
        </div>
        {subtitle && (
          <div className="session-item-subtitle">
            {subtitle}
          </div>
        )}
      </div>

      <div className="session-item-right">
        <span className={`session-item-time ${isStreaming ? 'session-item-time--now' : ''}`}>
          {timeLabel}
        </span>
        {actions}
      </div>
    </div>
  );
}
