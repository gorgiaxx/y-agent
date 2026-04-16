import type { ReactNode } from 'react';
import type { SessionInfo } from '../../types';
import { formatSessionRelativeTime } from '../chat-panel/sessionListActivity';
import './SessionItem.css';

export interface SessionItemProps {
  session: SessionInfo;
  isActive: boolean;
  isStreaming: boolean;
  subtitle?: string;
  actions?: ReactNode;
  className?: string;
  onClick: (e: React.MouseEvent) => void;
  onMouseDown?: (e: React.MouseEvent) => void;
  onMouseMove?: (e: React.MouseEvent) => void;
}

export function SessionItem({
  session,
  isActive,
  isStreaming,
  subtitle,
  actions,
  className = '',
  onClick,
  onMouseDown,
  onMouseMove,
}: SessionItemProps) {
  const timeLabel = formatSessionRelativeTime(session.updated_at, isStreaming);

  return (
    <div
      className={`session-item ${isActive ? 'session-item--active' : ''} ${isStreaming ? 'session-item--streaming' : ''} ${className}`}
      onClick={onClick}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
    >
      {isStreaming ? (
        <span className="session-item-spinner" aria-hidden="true" />
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
