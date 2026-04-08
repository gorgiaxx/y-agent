import { useState, useRef, useEffect, useMemo, useCallback, type MouseEvent as ReactMouseEvent } from 'react';
import {
  X,
  Plus,
  FolderOpen,
  MoreHorizontal,
  Pencil,
  Trash2,
  ChevronRight,
  GitBranch,
} from 'lucide-react';
import type { SessionInfo, WorkspaceInfo } from '../../types';
import { WorkspaceDialog } from './WorkspaceDialog';
import './ChatSidebarPanel.css';

interface ChatSidebarPanelProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  streamingSessionIds: Set<string>;
  workspaces: WorkspaceInfo[];
  sessionWorkspaceMap: Record<string, string>;
  onSelectSession: (id: string) => void;
  onNewChat: () => void;
  onNewChatInWorkspace: (workspaceId: string) => void;
  onDeleteSession: (id: string) => void;
  onForkSession?: (sessionId: string) => void;
  onCreateWorkspace: (name: string, path: string) => void;
  onUpdateWorkspace: (id: string, name: string, path: string) => void;
  onDeleteWorkspace: (id: string) => void;
  onAssignSession: (workspaceId: string, sessionId: string) => void;
  onUnassignSession: (sessionId: string) => void;
}

/** Return relative time string for a session item. */
function formatRelativeTime(iso: string, isStreaming: boolean): string {
  if (isStreaming) return 'now';
  const diff = Date.now() - new Date(iso).getTime();
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

export function ChatSidebarPanel({
  sessions,
  activeSessionId,
  streamingSessionIds,
  workspaces,
  sessionWorkspaceMap,
  onSelectSession,
  onNewChat,
  onNewChatInWorkspace,
  onDeleteSession,
  onForkSession,
  onCreateWorkspace,
  onUpdateWorkspace,
  onDeleteWorkspace,
  onAssignSession,
  onUnassignSession,
}: ChatSidebarPanelProps) {
  const COLLAPSED_STORAGE_KEY = 'y-gui:workspace-collapsed';
  const SESSION_ORDER_STORAGE_KEY = 'y-gui:session-order';

  const [searchQuery, setSearchQuery] = useState('');
  const [wsDialogOpen, setWsDialogOpen] = useState(false);
  const [editingWorkspace, setEditingWorkspace] = useState<WorkspaceInfo | null>(null);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [openSessionMenuId, setOpenSessionMenuId] = useState<string | null>(null);

  // -- Multi-select state --
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const lastClickedIdRef = useRef<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // -- Mouse-based reorder state (HTML5 DnD is blocked by Tauri webview) --
  const [draggedSessionId, setDraggedSessionId] = useState<string | null>(null);
  const [dragOverSessionId, setDragOverSessionId] = useState<string | null>(null);
  const [dragOverPosition, setDragOverPosition] = useState<'above' | 'below'>('above');
  const dragGroupRef = useRef<string[]>([]);
  /** Ref to track latest drop target for the mouseup handler (avoids stale closure). */
  const dropTargetRef = useRef<{ targetId: string; position: 'above' | 'below' } | null>(null);

  // Sorted workspaces by name (alphabetically).
  const sortedWorkspaces = useMemo(
    () => [...workspaces].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
    ),
    [workspaces],
  );

  // Workspace collapse state -- persisted in localStorage.
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(() => {
    try {
      const stored = localStorage.getItem(COLLAPSED_STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored) as string[];
        if (Array.isArray(parsed)) return new Set(parsed);
      }
    } catch { /* ignore corrupt data */ }
    // Fallback: collapse all except first workspace.
    return new Set(sortedWorkspaces.slice(1).map((w) => w.id));
  });

  // Persist collapse state on change.
  useEffect(() => {
    localStorage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify([...collapsedIds]));
  }, [collapsedIds]);

  const toggleCollapsed = (id: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Session order -- persisted in localStorage.
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

  // Persist session order on change.
  useEffect(() => {
    localStorage.setItem(SESSION_ORDER_STORAGE_KEY, JSON.stringify(sessionOrder));
  }, [sessionOrder]);

  /** Sort a list of sessions by user-defined order; unknowns go to the top sorted by updated_at desc. */
  const sortByUserOrder = useCallback(
    (list: SessionInfo[]): SessionInfo[] => {
      if (sessionOrder.length === 0) return list;
      const orderMap = new Map(sessionOrder.map((id, idx) => [id, idx]));
      return [...list].sort((a, b) => {
        const ia = orderMap.get(a.id);
        const ib = orderMap.get(b.id);
        // Both have custom positions -- sort by position.
        if (ia !== undefined && ib !== undefined) return ia - ib;
        // Items without position come first, sorted by updated_at desc.
        if (ia === undefined && ib === undefined) {
          return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
        }
        return ia === undefined ? -1 : 1;
      });
    },
    [sessionOrder],
  );

  // -- Mouse-based reorder (Tauri webview blocks HTML5 DnD by default) --

  /** Commit the reorder: move dragged session to the target position. */
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

  /** Called by React onMouseMove on each session item while dragging is active. */
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

  /** Initiate drag on mousedown with a 4px threshold to distinguish from clicks. */
  const handleMouseDown = useCallback(
    (e: React.MouseEvent, sessionId: string, groupSessionIds: string[]) => {
      if (e.button !== 0) return;
      if ((e.target as HTMLElement).closest('.btn-session-action, .session-context-menu')) return;

      const startX = e.clientX;
      const startY = e.clientY;
      let dragging = false;

      const onMove = (me: MouseEvent) => {
        if (!dragging) {
          const dx = me.clientX - startX;
          const dy = me.clientY - startY;
          if (Math.abs(dx) + Math.abs(dy) < 4) return;
          dragging = true;
          dragGroupRef.current = groupSessionIds;
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

        // Commit reorder using the latest drop target tracked by handleItemHover.
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
    [commitReorder],
  );

  // Close menus when clicking outside.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpenMenuId(null);
        setOpenSessionMenuId(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => {
      document.removeEventListener('mousedown', handler);
      document.body.classList.remove('y-gui-dragging');
    };
  }, []);

  const filtered = useMemo(() => {
    if (!searchQuery) return sessions;
    const q = searchQuery.toLowerCase();
    return sessions.filter(
      (s) => s.title?.toLowerCase().includes(q) || s.id.toLowerCase().includes(q),
    );
  }, [sessions, searchQuery]);

  // Group sessions by workspace, sorted alphabetically; apply user-defined order within groups.
  const { groups, ungrouped } = useMemo(() => {
    const g: { workspace: WorkspaceInfo | null; sessions: SessionInfo[] }[] = sortedWorkspaces.map(
      (ws) => ({
        workspace: ws,
        sessions: sortByUserOrder(
          filtered.filter((s) => sessionWorkspaceMap[s.id] === ws.id),
        ),
      }),
    );
    const u = sortByUserOrder(filtered.filter((s) => !sessionWorkspaceMap[s.id]));
    return { groups: g, ungrouped: u };
  }, [sortedWorkspaces, filtered, sessionWorkspaceMap, sortByUserOrder]);

  // Flat ordered list of all visible session IDs (for shift-range selection).
  const flatVisibleIds = useMemo(() => {
    const ids: string[] = [];
    for (const { workspace, sessions: wsSessions } of groups) {
      if (workspace && collapsedIds.has(workspace.id)) continue;
      for (const s of wsSessions) ids.push(s.id);
    }
    for (const s of ungrouped) ids.push(s.id);
    return ids;
  }, [groups, ungrouped, collapsedIds]);

  // Prune stale selected IDs when sessions list changes.
  useEffect(() => {
    const currentIds = new Set(sessions.map((s) => s.id));
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSelectedIds((prev) => {
      const next = new Set([...prev].filter((id) => currentIds.has(id)));
      if (next.size === prev.size) return prev;
      return next;
    });
  }, [sessions]);

  // Session item click handler with shift/ctrl multi-select.
  const handleSessionClick = useCallback(
    (e: ReactMouseEvent, sessionId: string) => {
      if (e.shiftKey && lastClickedIdRef.current) {
        // Range select between lastClickedId and current.
        const anchorIdx = flatVisibleIds.indexOf(lastClickedIdRef.current);
        const currentIdx = flatVisibleIds.indexOf(sessionId);
        if (anchorIdx !== -1 && currentIdx !== -1) {
          const start = Math.min(anchorIdx, currentIdx);
          const end = Math.max(anchorIdx, currentIdx);
          const rangeIds = flatVisibleIds.slice(start, end + 1);
          setSelectedIds((prev) => {
            const next = new Set(prev);
            for (const id of rangeIds) next.add(id);
            return next;
          });
        }
        return; // Do not navigate on shift-click.
      }

      if (e.ctrlKey || e.metaKey) {
        // Toggle individual item.
        setSelectedIds((prev) => {
          const next = new Set(prev);
          if (next.has(sessionId)) next.delete(sessionId);
          else next.add(sessionId);
          return next;
        });
        lastClickedIdRef.current = sessionId;
        return; // Do not navigate on ctrl/cmd-click.
      }

      // Plain click: clear selection and navigate.
      if (selectedIds.size > 0) {
        setSelectedIds(new Set());
      }
      lastClickedIdRef.current = sessionId;
      onSelectSession(sessionId);
    },
    [flatVisibleIds, selectedIds.size, onSelectSession],
  );

  // Batch delete all selected sessions.
  const handleBatchDelete = useCallback(() => {
    const ids = [...selectedIds];
    setSelectedIds(new Set());
    lastClickedIdRef.current = null;
    for (const id of ids) {
      onDeleteSession(id);
    }
  }, [selectedIds, onDeleteSession]);

  const getPreviewList = useCallback(
    (list: SessionInfo[]) => {
      if (!draggedSessionId || !dragOverSessionId || draggedSessionId === dragOverSessionId) {
        return list;
      }
      const sourceIdx = list.findIndex((s) => s.id === draggedSessionId);
      const targetIdx = list.findIndex((s) => s.id === dragOverSessionId);

      if (sourceIdx === -1 || targetIdx === -1) return list;

      const result = [...list];
      const [sourceItem] = result.splice(sourceIdx, 1);
      const newTargetIdx = result.findIndex((s) => s.id === dragOverSessionId);
      const insertIdx = dragOverPosition === 'below' ? newTargetIdx + 1 : newTargetIdx;
      result.splice(insertIdx, 0, sourceItem);
      return result;
    },
    [draggedSessionId, dragOverSessionId, dragOverPosition]
  );

  const renderSessionItem = (session: SessionInfo, groupSessionIds: string[]) => {
    const isStreaming = streamingSessionIds.has(session.id);
    const isActive = session.id === activeSessionId;
    const isSelected = selectedIds.has(session.id);
    const timeLabel = formatRelativeTime(session.updated_at, isStreaming);
    const assignedWs = workspaces.find((w) => w.id === sessionWorkspaceMap[session.id]);
    const isDragging = draggedSessionId === session.id;

    return (
      <div
        key={session.id}
        data-session-id={session.id}
        className={
          `session-item`
          + (isActive ? ' active' : '')
          + (isStreaming ? ' streaming' : '')
          + (isSelected ? ' session-item--selected' : '')
          + (isDragging ? ' session-item--dragging' : '')
        }
        onClick={(e) => handleSessionClick(e, session.id)}
        onMouseDown={(e) => handleMouseDown(e, session.id, groupSessionIds)}
        onMouseMove={(e) => handleItemHover(e, session.id)}
      >
        {/* Left: spinner or spacer */}
        {isStreaming ? (
          <span className="session-spinner" aria-hidden="true" />
        ) : (
          <span className="session-spinner-placeholder" />
        )}

        {/* Title */}
        <div className="session-item-title">
          {session.title || 'Untitled Session'}
        </div>

        {/* Right: timestamp + delete */}
        <div className="session-item-right">
          <span className={`session-time ${isStreaming ? 'session-time--now' : ''}`}>
            {timeLabel}
          </span>
          <button
            className="btn-session-action"
            onClick={(e) => {
              e.stopPropagation();
              setOpenSessionMenuId(openSessionMenuId === session.id ? null : session.id);
            }}
            title="Session actions"
          >
            <MoreHorizontal size={12} />
          </button>
        </div>

        {/* Session context menu */}
        {openSessionMenuId === session.id && (
          <div className="session-context-menu" ref={menuRef} onClick={(e) => e.stopPropagation()}>
            {workspaces.length > 0 && (
              <>
                <div className="context-menu-section">Move to workspace</div>
                {workspaces.map((ws) => (
                  <button
                    key={ws.id}
                    className="context-menu-item"
                    onClick={() => {
                      onAssignSession(ws.id, session.id);
                      setOpenSessionMenuId(null);
                    }}
                  >
                    <FolderOpen size={11} />
                    {ws.name}
                    {assignedWs?.id === ws.id && <span className="context-menu-check">*</span>}
                  </button>
                ))}
                {assignedWs && (
                  <button
                    className="context-menu-item"
                    onClick={() => {
                      onUnassignSession(session.id);
                      setOpenSessionMenuId(null);
                    }}
                  >
                    <X size={11} />
                    Remove from workspace
                  </button>
                )}
                <hr className="context-menu-divider" />
              </>
            )}
            {onForkSession && (
              <>
                <button
                  className="context-menu-item"
                  onClick={() => {
                    onForkSession(session.id);
                    setOpenSessionMenuId(null);
                  }}
                >
                  <GitBranch size={11} />
                  Fork session
                </button>
                <hr className="context-menu-divider" />
              </>
            )}
            <button
              className="context-menu-item context-menu-item--danger"
              onClick={() => {
                onDeleteSession(session.id);
                setOpenSessionMenuId(null);
              }}
            >
              <Trash2 size={11} />
              Delete session
            </button>
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      {/* Header */}
      <div className="sidebar-header">
        {/* Search */}
        <div className="sidebar-search">
          <input
            type="text"
            placeholder="Search sessions..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="search-input"
          />
        </div>
        <div className="sidebar-header-actions">
          <button
            className="btn-new-chat"
            onClick={() => setWsDialogOpen(true)}
            title="New Workspace"
          >
            <FolderOpen size={15} />
          </button>
          <button className="btn-new-chat" onClick={onNewChat} title="New Chat">
            <Plus size={16} />
          </button>
        </div>
      </div>

      {/* Session list grouped by workspace */}
      <div className="session-list">
        {/* Workspace sections */}
        {groups.map(({ workspace, sessions: originalWsSessions }) => {
          if (!workspace) return null;
          const isCollapsed = collapsedIds.has(workspace.id);
          const wsSessions = getPreviewList(originalWsSessions);
          return (
            <div key={workspace.id} className="workspace-section">
              <div
                className="workspace-label"
                onMouseLeave={() => setOpenMenuId(null)}
              >
                <button
                  className="btn-workspace-collapse"
                  onClick={() => toggleCollapsed(workspace.id)}
                  title={isCollapsed ? 'Expand' : 'Collapse'}
                  aria-expanded={!isCollapsed}
                >
                  <ChevronRight
                    size={12}
                    className={`workspace-chevron ${isCollapsed ? '' : 'workspace-chevron--open'}`}
                  />
                </button>
                <FolderOpen size={11} className="workspace-icon" />
                <span
                  className="workspace-name"
                  title={workspace.path}
                  onClick={() => toggleCollapsed(workspace.id)}
                  style={{ cursor: 'pointer' }}
                >
                  {workspace.name}
                </span>
                <button
                  className="btn-workspace-menu"
                  onClick={(e) => {
                    e.stopPropagation();
                    onNewChatInWorkspace(workspace.id);
                  }}
                  title="New session in this workspace"
                >
                  <Plus size={12} />
                </button>
                <button
                  className="btn-workspace-menu"
                  onClick={(e) => {
                    e.stopPropagation();
                    setOpenMenuId(openMenuId === workspace.id ? null : workspace.id);
                  }}
                  title="Workspace options"
                >
                  <MoreHorizontal size={12} />
                </button>
                {openMenuId === workspace.id && (
                  <div className="workspace-context-menu" ref={menuRef}>
                    <button
                      className="context-menu-item"
                      onClick={() => {
                        setEditingWorkspace(workspace);
                        setOpenMenuId(null);
                      }}
                    >
                      <Pencil size={11} />
                      Rename
                    </button>
                    <button
                      className="context-menu-item context-menu-item--danger"
                      onClick={() => {
                        onDeleteWorkspace(workspace.id);
                        setOpenMenuId(null);
                      }}
                    >
                      <Trash2 size={11} />
                      Delete workspace
                    </button>
                  </div>
                )}
              </div>
              {!isCollapsed && wsSessions.map((s) => renderSessionItem(s, originalWsSessions.map((x) => x.id)))}
            </div>
          );
        })}

        {/* Ungrouped sessions */}
        {ungrouped.length > 0 && (
          <div className="workspace-section">
            {workspaces.length > 0 && (
              <div className="workspace-label workspace-label--general">
                <span className="workspace-name">General</span>
                <button
                  className="btn-workspace-menu"
                  onClick={(e) => {
                    e.stopPropagation();
                    onNewChat();
                  }}
                  title="New session in General"
                >
                  <Plus size={12} />
                </button>
              </div>
            )}
            {getPreviewList(ungrouped).map((s) => renderSessionItem(s, ungrouped.map((x) => x.id)))}
          </div>
        )}

        {filtered.length === 0 && (
          <div className="session-empty">
            {searchQuery ? 'No matching sessions' : 'No sessions yet'}
          </div>
        )}

        {/* Batch action bar */}
        {selectedIds.size > 0 && (
          <div className="batch-action-bar">
            <span className="batch-action-count">{selectedIds.size} selected</span>
            <button
              className="batch-action-btn batch-action-btn--delete"
              onClick={handleBatchDelete}
            >
              <Trash2 size={13} />
              Delete
            </button>
          </div>
        )}
      </div>

      {/* Workspace creation dialog */}
      {wsDialogOpen && (
        <WorkspaceDialog
          onConfirm={(name, path) => {
            onCreateWorkspace(name, path);
            setWsDialogOpen(false);
          }}
          onClose={() => setWsDialogOpen(false)}
        />
      )}

      {/* Workspace edit dialog */}
      {editingWorkspace && (
        <WorkspaceDialog
          initialName={editingWorkspace.name}
          initialPath={editingWorkspace.path}
          onConfirm={(name, path) => {
            onUpdateWorkspace(editingWorkspace.id, name, path);
            setEditingWorkspace(null);
          }}
          onClose={() => setEditingWorkspace(null)}
        />
      )}
    </>
  );
}
