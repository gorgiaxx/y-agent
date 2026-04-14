import { Plus, Settings2, Trash2 } from 'lucide-react';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { ScrollArea } from '../ui/ScrollArea';

interface AgentSessionRailProps {
  sessions: Array<{
    id: string;
    title: string | null;
    message_count: number;
  }>;
  activeSessionId: string | null;
  onEdit: () => void;
  onNewSession: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
}

export function AgentSessionRail({
  sessions,
  activeSessionId,
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
        <Badge variant="outline">{sessions.length}</Badge>
      </div>

      <ScrollArea className="flex-1 min-h-0">
        <div className="agents-session-list">
          {sessions.map((session) => (
            <div
              key={session.id}
              className={[
                'agents-session-item',
                session.id === activeSessionId ? 'agents-session-item--active' : '',
              ].filter(Boolean).join(' ')}
            >
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

          {sessions.length === 0 && (
            <div className="agents-session-empty">No sessions yet.</div>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}
