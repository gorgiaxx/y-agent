import { useMemo } from 'react';
import { ArrowLeft, Loader2, Plus, Settings2, Trash2 } from 'lucide-react';
import { NavSidebar, NavItem, NavDivider } from '../common/NavSidebar';
import { SessionItem } from '../shared/SessionItem';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import type { SessionInfo } from '../../types';
import { useSessionDragReorder } from '../../hooks/useSessionDragReorder';
import { STORAGE_KEYS } from '../../constants/storageKeys';

interface AgentStudioSidebarNavProps {
  agentName: string;
  sessions: SessionInfo[];
  activeSessionId: string | null;
  loading: boolean;
  streamingSessionIds: Set<string>;
  onBack: () => void;
  onEdit: () => void;
  onNewSession: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
}

export function AgentStudioSidebarNav({
  agentName,
  sessions,
  activeSessionId,
  loading,
  streamingSessionIds,
  onBack,
  onEdit,
  onNewSession,
  onSelectSession,
  onDeleteSession,
}: AgentStudioSidebarNavProps) {
  const {
    draggedSessionId,
    handleItemHover,
    handleMouseDown,
    getPreviewList,
    sortByUserOrder,
  } = useSessionDragReorder({
    sessions,
    storageKey: STORAGE_KEYS.AGENT_SESSION_ORDER,
  });

  const sortedSessions = useMemo(
    () => sortByUserOrder(sessions),
    [sessions, sortByUserOrder],
  );

  const allSessionIds = useMemo(
    () => sortedSessions.map((s) => s.id),
    [sortedSessions],
  );

  const displaySessions = useMemo(
    () => getPreviewList(sortedSessions),
    [getPreviewList, sortedSessions],
  );

  return (
    <NavSidebar
      footer={
        <NavItem
          icon={<Settings2 size={15} />}
          label="Edit Agent"
          onClick={onEdit}
        />
      }
    >
      <NavItem
        icon={<ArrowLeft size={15} />}
        label={agentName}
        onClick={onBack}
      />
      <NavDivider />

      <div className="session-list-general">
        <div className="agent-session-toolbar">
          <div className="agent-session-toolbar-label">
            <span>Sessions</span>
            <div className="agent-session-toolbar-meta">
              {loading && (
                <span className="agent-session-toolbar-loading" aria-label="Loading sessions">
                  <Loader2 size={12} className="agents-spin" />
                </span>
              )}
              <Badge variant="outline">{sessions.length}</Badge>
            </div>
          </div>
          <Button variant="icon" size="sm" onClick={onNewSession} title="New Session">
            <Plus size={14} />
          </Button>
        </div>

        <div className="session-pane">
          {loading && sessions.length === 0 && (
            <div className="session-empty">
              <Loader2 size={14} className="agents-spin" />
              <span>Loading sessions...</span>
            </div>
          )}

          {displaySessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              isActive={session.id === activeSessionId}
              isStreaming={streamingSessionIds.has(session.id)}
              subtitle={`${session.message_count} messages`}
              className={draggedSessionId === session.id ? 'session-item--dragging' : ''}
              onClick={() => onSelectSession(session.id)}
              onMouseDown={(e) => handleMouseDown(e, session.id, allSessionIds)}
              onMouseMove={(e) => handleItemHover(e, session.id)}
              actions={
                <Button
                  variant="icon"
                  size="sm"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDeleteSession(session.id);
                  }}
                  title="Delete session"
                >
                  <Trash2 size={10} />
                </Button>
              }
            />
          ))}

          {!loading && sessions.length === 0 && (
            <div className="session-empty">No sessions yet.</div>
          )}
        </div>
      </div>
    </NavSidebar>
  );
}
