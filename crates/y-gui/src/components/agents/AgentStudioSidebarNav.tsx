import { useState, useRef, useCallback, useMemo, useEffect } from 'react';
import { ArrowLeft, Loader2, Plus, Settings2, Trash2 } from 'lucide-react';
import { NavSidebar, NavItem, NavDivider } from '../common/NavSidebar';
import { SessionItem } from '../shared/SessionItem';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import type { SessionInfo } from '../../types';

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

const SESSION_ORDER_STORAGE_KEY = 'y-gui:agent-session-order';

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
  const [draggedSessionId, setDraggedSessionId] = useState<string | null>(null);
  const [dragOverSessionId, setDragOverSessionId] = useState<string | null>(null);
  const [dragOverPosition, setDragOverPosition] = useState<'above' | 'below'>('above');
  const dragGroupRef = useRef<string[]>([]);
  const dropTargetRef = useRef<{ targetId: string; position: 'above' | 'below' } | null>(null);

  const [sessionOrder, setSessionOrder] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem(SESSION_ORDER_STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored) as string[];
        if (Array.isArray(parsed)) return parsed;
      }
    } catch { /* ignore corrupt data */ }
    return [];
  });

  useEffect(() => {
    localStorage.setItem(SESSION_ORDER_STORAGE_KEY, JSON.stringify(sessionOrder));
  }, [sessionOrder]);

  const sortByUserOrder = useCallback(
    (list: SessionInfo[]): SessionInfo[] => {
      if (sessionOrder.length === 0) return list;
      const orderMap = new Map(sessionOrder.map((id, idx) => [id, idx]));
      return [...list].sort((a, b) => {
        const ia = orderMap.get(a.id);
        const ib = orderMap.get(b.id);
        if (ia !== undefined && ib !== undefined) return ia - ib;
        if (ia === undefined && ib === undefined) {
          return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
        }
        return ia === undefined ? -1 : 1;
      });
    },
    [sessionOrder],
  );

  const sortedSessions = useMemo(
    () => sortByUserOrder(sessions),
    [sessions, sortByUserOrder],
  );

  const allSessionIds = useMemo(
    () => sortedSessions.map((s) => s.id),
    [sortedSessions],
  );

  const commitReorder = useCallback(
    (sourceId: string, targetId: string, dropPos: 'above' | 'below', groupSessionIds: string[]) => {
      if (sourceId === targetId) return;
      if (!groupSessionIds.includes(sourceId)) return;

      const newGroupOrder = groupSessionIds.filter((id) => id !== sourceId);
      const targetIdx = newGroupOrder.indexOf(targetId);
      if (targetIdx === -1) return;
      const insertIdx = dropPos === 'below' ? targetIdx + 1 : targetIdx;
      newGroupOrder.splice(insertIdx, 0, sourceId);

      const allIds = sessions.map((s) => s.id);
      const currentOrder = sessionOrder.length > 0
        ? [...sessionOrder, ...allIds.filter((id) => !sessionOrder.includes(id))]
        : [...allIds];
      const groupSet = new Set(groupSessionIds);
      const firstGroupPos = currentOrder.findIndex((id) => groupSet.has(id));
      const withoutGroup = currentOrder.filter((id) => !groupSet.has(id));
      withoutGroup.splice(firstGroupPos, 0, ...newGroupOrder);
      setSessionOrder(withoutGroup);
    },
    [sessions, sessionOrder],
  );

  const handleItemHover = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      if (!draggedSessionId || draggedSessionId === sessionId) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const pos: 'above' | 'below' = e.clientY < rect.top + rect.height / 2 ? 'above' : 'below';
      setDragOverSessionId(sessionId);
      setDragOverPosition(pos);
      dropTargetRef.current = { targetId: sessionId, position: pos };
    },
    [draggedSessionId],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      if (e.button !== 0) return;
      if ((e.target as HTMLElement).closest('.button--icon')) return;

      const startX = e.clientX;
      const startY = e.clientY;
      let dragging = false;

      const onMove = (me: MouseEvent) => {
        if (!dragging) {
          const dx = me.clientX - startX;
          const dy = me.clientY - startY;
          if (Math.abs(dx) + Math.abs(dy) < 4) return;
          dragging = true;
          dragGroupRef.current = allSessionIds;
          dropTargetRef.current = null;
          setDraggedSessionId(sessionId);
          document.body.classList.add('y-gui-dragging');
        }
      };

      const onUp = () => {
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        document.body.classList.remove('y-gui-dragging');
        if (!dragging) return;

        const target = dropTargetRef.current;
        if (target) {
          commitReorder(sessionId, target.targetId, target.position, dragGroupRef.current);
        }

        dropTargetRef.current = null;
        setDraggedSessionId(null);
        setDragOverSessionId(null);
      };

      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    },
    [commitReorder, allSessionIds],
  );

  const displaySessions = useMemo(() => {
    if (!draggedSessionId || !dragOverSessionId || draggedSessionId === dragOverSessionId) {
      return sortedSessions;
    }
    const sourceIdx = sortedSessions.findIndex((s) => s.id === draggedSessionId);
    const targetIdx = sortedSessions.findIndex((s) => s.id === dragOverSessionId);
    if (sourceIdx === -1 || targetIdx === -1) return sortedSessions;

    const result = [...sortedSessions];
    const [sourceItem] = result.splice(sourceIdx, 1);
    const newTargetIdx = result.findIndex((s) => s.id === dragOverSessionId);
    const insertIdx = dragOverPosition === 'below' ? newTargetIdx + 1 : newTargetIdx;
    result.splice(insertIdx, 0, sourceItem);
    return result;
  }, [sortedSessions, draggedSessionId, dragOverSessionId, dragOverPosition]);

  useEffect(() => {
    return () => {
      document.body.classList.remove('y-gui-dragging');
    };
  }, []);

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
              onMouseDown={(e) => handleMouseDown(e, session.id)}
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
