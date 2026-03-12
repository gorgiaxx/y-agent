import { useState } from 'react';
import type { SessionInfo } from '../types';
import './Sidebar.css';

interface SidebarProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  onSelectSession: (id: string) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;
}

export function Sidebar({
  sessions,
  activeSessionId,
  onSelectSession,
  onNewChat,
  onDeleteSession,
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState('');

  const filtered = sessions.filter((s) => {
    if (!searchQuery) return true;
    const q = searchQuery.toLowerCase();
    return (
      s.title?.toLowerCase().includes(q) ||
      s.id.toLowerCase().includes(q)
    );
  });

  const formatDate = (iso: string) => {
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const days = Math.floor(diff / 86_400_000);
    if (days === 0) return 'Today';
    if (days === 1) return 'Yesterday';
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString();
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <h2 className="sidebar-title">Sessions</h2>
        <button className="btn-new-chat" onClick={onNewChat} title="New Chat">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
      </div>

      <div className="sidebar-search">
        <input
          type="text"
          placeholder="Search sessions..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="search-input"
        />
      </div>

      <div className="session-list">
        {filtered.length === 0 && (
          <div className="session-empty">
            {searchQuery ? 'No matching sessions' : 'No sessions yet'}
          </div>
        )}
        {filtered.map((session) => (
          <div
            key={session.id}
            className={`session-item ${session.id === activeSessionId ? 'active' : ''}`}
            onClick={() => onSelectSession(session.id)}
          >
            <div className="session-item-title">
              {session.title || 'Untitled Session'}
            </div>
            <div className="session-item-meta">
              <span className="session-date">{formatDate(session.updated_at)}</span>
              <button
                className="btn-delete-session"
                onClick={(e) => {
                  e.stopPropagation();
                  onDeleteSession(session.id);
                }}
                title="Delete session"
              >
                ×
              </button>
            </div>
          </div>
        ))}
      </div>
    </aside>
  );
}
