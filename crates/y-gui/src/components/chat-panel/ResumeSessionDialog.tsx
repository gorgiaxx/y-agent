import { useEffect, useMemo, useRef, useState } from 'react';
import {
  CornerDownLeft,
  Folder,
  History,
  LoaderCircle,
  MessageSquare,
  Search,
  X,
} from 'lucide-react';
import type { SessionInfo } from '../../types';
import { Dialog, DialogContent } from '../ui/Dialog';
import { formatSessionRelativeTime } from './sessionListActivity';
import { nextResumeSelection } from './resumeSessionDialogState';
import './ResumeSessionDialog.css';

interface ResumeSessionDialogProps {
  open: boolean;
  workspacePath: string | null;
  currentSessionId: string | null;
  sessions: SessionInfo[];
  loading: boolean;
  error: string | null;
  onSelect: (sessionId: string) => void;
  onClose: () => void;
}

type ResumeSessionDialogContentProps = Omit<ResumeSessionDialogProps, 'open'>;

function workspaceName(path: string | null): string {
  if (!path) return 'No workspace selected';
  const parts = path.replaceAll('\\', '/').split('/').filter(Boolean);
  return parts.at(-1) ?? path;
}

function sessionTitle(session: SessionInfo): string {
  return session.manual_title ?? session.title ?? 'Untitled';
}

export function ResumeSessionDialogContent({
  workspacePath,
  currentSessionId,
  sessions,
  loading,
  error,
  onSelect,
  onClose,
}: ResumeSessionDialogContentProps) {
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(() => {
    const currentIndex = sessions.findIndex((session) => session.id === currentSessionId);
    return Math.max(0, currentIndex);
  });
  const visibleSessions = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return sessions;
    return sessions.filter((session) => (
      sessionTitle(session).toLowerCase().includes(normalized)
      || session.id.toLowerCase().includes(normalized)
    ));
  }, [query, sessions]);
  const activeIndex = Math.min(selectedIndex, Math.max(0, visibleSessions.length - 1));

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>('.resume-session-item--selected')
      ?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex]);

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const navigationKey = event.key;
      event.preventDefault();
      setSelectedIndex((index) => nextResumeSelection(
        Math.min(index, Math.max(0, visibleSessions.length - 1)),
        visibleSessions.length,
        navigationKey,
      ));
      return;
    }
    if (event.key === 'Enter' && visibleSessions[activeIndex]) {
      event.preventDefault();
      onSelect(visibleSessions[activeIndex].id);
    }
  };

  return (
    <div className="resume-dialog" onKeyDown={handleKeyDown}>
      <header className="resume-dialog-header">
        <div className="resume-dialog-heading">
          <span className="resume-dialog-icon" aria-hidden="true">
            <History size={16} />
          </span>
          <div>
            <h2 id="resume-session-dialog-title" className="resume-dialog-title">
              Resume session
            </h2>
            <p className="resume-dialog-subtitle">Continue work from this workspace</p>
          </div>
        </div>
        <button
          type="button"
          className="resume-dialog-close"
          onClick={onClose}
          aria-label="Close resume session picker"
        >
          <X size={16} />
        </button>
      </header>

      <div className="resume-dialog-workspace" title={workspacePath ?? undefined}>
        <Folder size={13} aria-hidden="true" />
        <span className="resume-dialog-workspace-name">{workspaceName(workspacePath)}</span>
        {workspacePath && (
          <span className="resume-dialog-workspace-path">{workspacePath}</span>
        )}
      </div>

      <label className="resume-dialog-search">
        <Search size={14} aria-hidden="true" />
        <input
          ref={searchRef}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
          }}
          placeholder="Search by title or session ID"
          aria-label="Search sessions"
        />
        {!loading && !error && (
          <span className="resume-dialog-result-count">{visibleSessions.length}</span>
        )}
      </label>

      <div className="resume-session-list" ref={listRef} role="listbox">
        {loading && (
          <div className="resume-dialog-state">
            <LoaderCircle className="resume-dialog-busy-icon" size={18} />
            <span>Loading sessions...</span>
          </div>
        )}
        {!loading && error && (
          <div className="resume-dialog-state resume-dialog-state--error">{error}</div>
        )}
        {!loading && !error && visibleSessions.length === 0 && (
          <div className="resume-dialog-state">
            <History size={20} />
            <span>No sessions found in this workspace.</span>
          </div>
        )}
        {!loading && !error && visibleSessions.map((session, index) => {
          const isSelected = index === activeIndex;
          const isCurrent = session.id === currentSessionId;
          const messageLabel = `${session.message_count} ${session.message_count === 1 ? 'message' : 'messages'}`;
          return (
            <button
              type="button"
              key={session.id}
              role="option"
              aria-selected={isSelected}
              className={[
                'resume-session-item',
                isSelected ? 'resume-session-item--selected' : '',
                isCurrent ? 'resume-session-item--current' : '',
              ].filter(Boolean).join(' ')}
              onMouseEnter={() => setSelectedIndex(index)}
              onClick={() => onSelect(session.id)}
            >
              <span className="resume-session-item-main">
                <span className="resume-session-item-title-row">
                  <span className="resume-session-item-title">{sessionTitle(session)}</span>
                  {isCurrent && <span className="resume-session-current-badge">Current</span>}
                </span>
                <span className="resume-session-item-meta">
                  <span><MessageSquare size={11} />{messageLabel}</span>
                  <span>{formatSessionRelativeTime(session.updated_at, false)}</span>
                  <span className="resume-session-item-id">{session.id.slice(0, 8)}</span>
                </span>
              </span>
              <CornerDownLeft
                size={14}
                className="resume-session-enter-icon"
                aria-hidden="true"
              />
            </button>
          );
        })}
      </div>

      <footer className="resume-dialog-footer">
        <span><kbd>↑</kbd><kbd>↓</kbd> Navigate</span>
        <span><kbd>Enter</kbd> Resume</span>
        <span><kbd>Esc</kbd> Close</span>
      </footer>
    </div>
  );
}

export function ResumeSessionDialog({ open, ...props }: ResumeSessionDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(nextOpen) => { if (!nextOpen) props.onClose(); }}>
      <DialogContent
        size="md"
        className="resume-dialog-shell"
        aria-labelledby="resume-session-dialog-title"
      >
        <ResumeSessionDialogContent {...props} />
      </DialogContent>
    </Dialog>
  );
}
