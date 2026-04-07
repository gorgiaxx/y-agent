// RewindPanel -- modal overlay listing available rewind points.
//
// Shown when user triggers /rewind or double-taps Escape.
// Each point shows the user message preview, timestamp, and
// diff stats (files changed/created). Clicking a point
// executes the rewind and reloads messages.

import { useCallback, useEffect, useRef } from 'react';
import { X, RotateCcw, FileEdit, FilePlus, Loader2 } from 'lucide-react';
import type { RewindPointInfo } from '../../hooks/useRewind';
import './RewindPanel.css';

interface RewindPanelProps {
  points: RewindPointInfo[];
  isLoading: boolean;
  isRewinding: boolean;
  error: string | null;
  /** Called with the full point info so the parent can extract message content. */
  onSelect: (point: RewindPointInfo) => void;
  onClose: () => void;
}

/** Format a Unix timestamp to a relative time string. */
function formatTimestamp(ts: number): string {
  const diff = Date.now() - ts * 1000;
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export function RewindPanel({
  points,
  isLoading,
  isRewinding,
  error,
  onSelect,
  onClose,
}: RewindPanelProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  // Close on Escape key.
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  // Auto-focus the panel for keyboard accessibility.
  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  return (
    <div className="rewind-backdrop" onClick={handleBackdropClick}>
      <div className="rewind-panel" ref={panelRef} tabIndex={-1}>
        {/* Header */}
        <div className="rewind-header">
          <div className="rewind-header-left">
            <RotateCcw size={16} className="rewind-header-icon" />
            <h3 className="rewind-title">Rewind</h3>
          </div>
          <button className="rewind-close" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </div>

        <p className="rewind-subtitle">
          Select a message to rewind to. Files will be restored to their state
          at that point.
        </p>

        {/* Error */}
        {error && <div className="rewind-error">{error}</div>}

        {/* Loading */}
        {isLoading && (
          <div className="rewind-loading">
            <Loader2 size={20} className="rewind-spinner" />
            <span>Loading rewind points...</span>
          </div>
        )}

        {/* Rewinding overlay */}
        {isRewinding && (
          <div className="rewind-loading">
            <Loader2 size={20} className="rewind-spinner" />
            <span>Rewinding...</span>
          </div>
        )}

        {/* Points list */}
        {!isLoading && !isRewinding && points.length === 0 && !error && (
          <div className="rewind-empty">
            No rewind points available. File changes are tracked automatically
            as you chat.
          </div>
        )}

        {!isLoading && !isRewinding && points.length > 0 && (
          <div className="rewind-list">
            {points.map((point) => (
              <button
                key={point.message_id}
                className="rewind-point"
                onClick={() => onSelect(point)}
              >
                <div className="rewind-point-header">
                  <span className="rewind-point-turn">
                    Turn {point.turn_number}
                  </span>
                  <span className="rewind-point-time">
                    {formatTimestamp(point.timestamp)}
                  </span>
                </div>
                <div className="rewind-point-preview">
                  {point.message_preview}
                </div>
                <div className="rewind-point-stats">
                  {point.diff_stats.files_changed > 0 && (
                    <span className="rewind-stat rewind-stat--changed">
                      <FileEdit size={12} />
                      {point.diff_stats.files_changed} changed
                    </span>
                  )}
                  {point.diff_stats.files_created > 0 && (
                    <span className="rewind-stat rewind-stat--created">
                      <FilePlus size={12} />
                      {point.diff_stats.files_created} created
                    </span>
                  )}
                </div>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
