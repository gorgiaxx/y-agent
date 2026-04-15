import { Loader2, Plus, Settings2, Trash2 } from 'lucide-react';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { ScrollArea } from '../ui/ScrollArea';
import type { SessionInfo } from '../../types';
import { formatSessionRelativeTime } from '../chat-panel/sessionListActivity';

interface AgentSessionRailProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  loading: boolean;
  streamingSessionIds: Set<string>;
  onEdit: () => void;
  onNewSession: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
}

export function AgentSessionRail({
  sessions,
  activeSessionId,
  loading,
  streamingSessionIds,
  onEdit,
  onNewSession,
  onSelectSession,
  onDeleteSession,
}: AgentSessionRailProps) {
  return (
    <aside className="agents-session-rail">
      <div className="agents-session-rail-actions">
        <Button variant="icon" size="sm" onClick={onEdit} title="Edit Agent">
          <Settings2 size={14} />
        </Button>
        <Button variant="icon" size="sm" onClick={onNewSession} title="New Session">
          <Plus size={14} />
        </Button>
      </div>

      <div className="agents-session-list-header">
        <span>Sessions</span>
        <div className="agents-session-list-header-meta">
          {loading && (
            <span className="agents-session-list-loading" aria-label="Loading sessions">
              <Loader2 size={12} className="agents-spin" />
            </span>
          )}
          <Badge variant="outline">{sessions.length}</Badge>
        </div>
      </div>

      <ScrollArea className="flex-1 min-h-0">
        <div className="agents-session-list">
          {loading && sessions.length === 0 && (
            <div className="agents-session-empty">
              <Loader2 size={14} className="agents-spin" />
              <span>Loading sessions...</span>
            </div>
          )}

          {sessions.map((session) => (
            <div
              key={session.id}
              className={[
                'agents-session-item',
                session.id === activeSessionId ? 'agents-session-item--active' : '',
                streamingSessionIds.has(session.id) ? 'agents-session-item--streaming' : '',
              ].filter(Boolean).join(' ')}
            >
              {streamingSessionIds.has(session.id) ? (
                <span className="agents-session-item-spinner" aria-hidden="true" />
              ) : (
                <span className="agents-session-item-spinner-placeholder" aria-hidden="true" />
              )}

              <button
                type="button"
                className="agents-session-item-main"
                onClick={() => onSelectSession(session.id)}
              >
                <span className="agents-session-item-title">
                  {session.title || 'Untitled'}
                </span>
                <span className="agents-session-item-meta">
                  {session.message_count} messages
                </span>
              </button>

              <span
                className={[
                  'agents-session-item-time',
                  streamingSessionIds.has(session.id) ? 'agents-session-item-time--now' : '',
                ].filter(Boolean).join(' ')}
              >
                {formatSessionRelativeTime(session.updated_at, streamingSessionIds.has(session.id))}
              </span>

              <Button
                variant="icon"
                size="sm"
                onClick={() => onDeleteSession(session.id)}
                title="Delete session"
              >
                <Trash2 size={10} />
              </Button>
            </div>
          ))}

          {!loading && sessions.length === 0 && (
            <div className="agents-session-empty">No sessions yet.</div>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}
