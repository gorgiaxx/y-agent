import { useState, useRef, useEffect, useMemo, useCallback, type MouseEvent as ReactMouseEvent, type ReactElement } from 'react';
import { createPortal } from 'react-dom';
import {
  X,
  Plus,
  FolderOpen,
  Folder,
  FolderPlus,
  FolderSymlink,
  MoreHorizontal,
  Pencil,
  Trash2,
  GitBranch,
} from 'lucide-react';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import type { SessionInfo, WorkspaceInfo } from '../../types';
import { SessionItem } from '../shared/SessionItem';
import { WorkspaceDialog } from './WorkspaceDialog';
import { Button } from '../ui';
import { PanelToolbar, type SortField, type PanelToolbarAction } from '../common/PanelToolbar';
import {
  calculateFloatingMenuPosition,
  calculateWorkspaceOptionsMaxHeight,
  getSessionPaneLayout,
} from './chatSidebarLayout';
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
  onRenameSession: (id: string, title: string | null) => void;
  onCreateWorkspace: (name: string, path: string) => void;
  onUpdateWorkspace: (id: string, name: string, path: string) => void;
  onDeleteWorkspace: (id: string) => void;
  onAssignSession: (workspaceId: string, sessionId: string) => void;
  onUnassignSession: (sessionId: string) => void;
  /** Controlled search query; falls back to internal state when undefined. */
  searchQuery?: string;
  onSearchQueryChange?: (q: string) => void;
  /** Open the workspace creation dialog externally. */
  onRequestNewWorkspace?: () => void;
}

type OpenMenuState =
  | { kind: 'workspace'; id: string }
  | { kind: 'session'; id: string }
  | null;


function sortSessions(list: SessionInfo[], field: SortField): SessionInfo[] {
  if (field === 'default') return list;
  return [...list].sort((a, b) => {
    if (field === 'name') {
      return (a.manual_title || a.title || '').localeCompare(b.manual_title || b.title || '', undefined, { sensitivity: 'base' });
    }
    if (field === 'created') {
      return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
    }
    // updated / last active
    return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
  });
}

function sortWorkspaces(list: WorkspaceInfo[], field: SortField): WorkspaceInfo[] {
  if (field === 'default') return list;
  return [...list].sort((a, b) => {
    if (field === 'name') {
      return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
    }
    // created / updated not available on WorkspaceInfo; fall back to name
    return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
  });
}


// Component

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
  onRenameSession,
  onCreateWorkspace,
  onUpdateWorkspace,
  onDeleteWorkspace,
  onAssignSession,
  onUnassignSession,
  searchQuery: searchQueryProp,
  onSearchQueryChange: _onSearchQueryChange,
}: ChatSidebarPanelProps) {
  const COLLAPSED_STORAGE_KEY = 'y-gui:workspace-collapsed';
  const PINNED_STORAGE_KEY = 'y-gui:pinned-sessions';
  const WS_SORT_STORAGE_KEY = 'y-gui:workspace-sort';
  const SESSION_SORT_STORAGE_KEY = 'y-gui:session-sort';

  const [internalSearchQuery] = useState('');
  const searchQuery = searchQueryProp ?? internalSearchQuery;
  const [wsDialogOpen, setWsDialogOpen] = useState(false);
  const [editingWorkspace, setEditingWorkspace] = useState<WorkspaceInfo | null>(null);
  const [openMenu, setOpenMenu] = useState<OpenMenuState>(null);
  const [menuPosition, setMenuPosition] = useState<ReturnType<typeof calculateFloatingMenuPosition> | null>(null);

  // -- Multi-select state --
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const lastClickedIdRef = useRef<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuAnchorRef = useRef<HTMLElement | null>(null);

  // -- Mouse-based reorder state --
  const [draggedSessionId, setDraggedSessionId] = useState<string | null>(null);
  const [dragOverSessionId, setDragOverSessionId] = useState<string | null>(null);
  const [dragOverPosition, setDragOverPosition] = useState<'above' | 'below'>('above');
  const dragGroupRef = useRef<string[]>([]);

  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const renameInputRef = useRef<HTMLInputElement>(null);
  const dropTargetRef = useRef<{ targetId: string; position: 'above' | 'below' } | null>(null);

  // -- Sort state (persisted) --
  const [wsSortField, setWsSortField] = useState<SortField>(() => {
    try {
      const v = localStorage.getItem(WS_SORT_STORAGE_KEY) as SortField;
      if (v && ['default', 'name', 'created', 'updated'].includes(v)) return v;
    } catch { /* ignore */ }
    return 'default';
  });

  const [sessionSortField, setSessionSortField] = useState<SortField>(() => {
    try {
      const v = localStorage.getItem(SESSION_SORT_STORAGE_KEY) as SortField;
      if (v && ['default', 'name', 'created', 'updated'].includes(v)) return v;
    } catch { /* ignore */ }
    return 'default';
  });

  useEffect(() => { localStorage.setItem(WS_SORT_STORAGE_KEY, wsSortField); }, [wsSortField]);
  useEffect(() => { localStorage.setItem(SESSION_SORT_STORAGE_KEY, sessionSortField); }, [sessionSortField]);

  // -- Section collapse state (persisted) --
  const SECTION_COLLAPSED_KEY = 'y-gui:section-collapsed';
  const [sectionCollapsed, setSectionCollapsed] = useState<Record<string, boolean>>(() => {
    try {
      const stored = localStorage.getItem(SECTION_COLLAPSED_KEY);
      if (stored) {
        const parsed = JSON.parse(stored) as Record<string, boolean>;
        if (parsed && typeof parsed === 'object') return parsed;
      }
    } catch { /* ignore */ }
    return {};
  });

  useEffect(() => {
    localStorage.setItem(SECTION_COLLAPSED_KEY, JSON.stringify(sectionCollapsed));
  }, [sectionCollapsed]);

  const toggleSectionCollapsed = useCallback((section: string) => {
    setSectionCollapsed((prev) => ({ ...prev, [section]: !prev[section] }));
  }, []);

  // -- Pinned sessions state (persisted) --
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(() => {
    try {
      const stored = localStorage.getItem(PINNED_STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored) as string[];
        if (Array.isArray(parsed)) return new Set(parsed);
      }
    } catch { /* ignore */ }
    return new Set();
  });

  useEffect(() => {
    localStorage.setItem(PINNED_STORAGE_KEY, JSON.stringify([...pinnedIds]));
  }, [pinnedIds]);

  const togglePin = useCallback((sessionId: string) => {
    setPinnedIds((prev) => {
      const next = new Set(prev);
      if (next.has(sessionId)) next.delete(sessionId);
      else next.add(sessionId);
      return next;
    });
  }, []);

  // Prune pinned IDs for sessions that no longer exist.
  useEffect(() => {
    const currentIds = new Set(sessions.map((s) => s.id));
    setPinnedIds((prev) => {
      const next = new Set([...prev].filter((id) => currentIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [sessions]);

  // Sorted workspaces.
  const sortedWorkspaces = useMemo(
    () => sortWorkspaces([...workspaces], wsSortField),
    [workspaces, wsSortField],
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
    return new Set(sortedWorkspaces.slice(1).map((w) => w.id));
  });

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

  const allExpanded = sortedWorkspaces.length > 0 && sortedWorkspaces.every((w) => !collapsedIds.has(w.id));
  const toggleAllExpanded = () => {
    if (allExpanded) {
      setCollapsedIds(new Set(sortedWorkspaces.map((w) => w.id)));
    } else {
      setCollapsedIds(new Set());
    }
  };

  // Session order -- persisted in localStorage (for custom/drag reorder).
  const SESSION_ORDER_STORAGE_KEY = 'y-gui:session-order';
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

  /** Sort a list of sessions by user-defined order; unknowns go to the top sorted by updated_at desc. */
  const sortByUserOrder = useCallback(
    (list: SessionInfo[]): SessionInfo[] => {
      if (sessionSortField !== 'default') return sortSessions(list, sessionSortField);
      if (sessionOrder.length === 0) return sortSessions(list, 'updated');
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
    [sessionOrder, sessionSortField],
  );

  // -- Mouse-based reorder --

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
    (e: React.MouseEvent, sessionId: string, groupSessionIds: string[]) => {
      if (e.button !== 0) return;
      if ((e.target as HTMLElement).closest('.btn-session-action, .session-context-menu, .session-item-pin')) return;

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

  const closeOpenMenu = useCallback(() => {
    setOpenMenu(null);
    setMenuPosition(null);
    menuAnchorRef.current = null;
  }, []);

  const toggleMenu = useCallback(
    (nextMenu: NonNullable<OpenMenuState>, anchorElement: HTMLElement) => {
      if (openMenu?.kind === nextMenu.kind && openMenu.id === nextMenu.id) {
        closeOpenMenu();
        return;
      }

      menuAnchorRef.current = anchorElement;
      setMenuPosition(null);
      setOpenMenu(nextMenu);
    },
    [closeOpenMenu, openMenu],
  );

  // Close menus when clicking outside.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      const clickedMenu = menuRef.current?.contains(target) ?? false;
      const clickedAnchor = menuAnchorRef.current?.contains(target) ?? false;
      if (!clickedMenu && !clickedAnchor) {
        closeOpenMenu();
      }
    };

    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        closeOpenMenu();
      }
    };

    document.addEventListener('mousedown', handler);
    document.addEventListener('keydown', handleEscape);
    return () => {
      document.removeEventListener('mousedown', handler);
      document.removeEventListener('keydown', handleEscape);
      document.body.classList.remove('y-gui-dragging');
    };
  }, [closeOpenMenu]);

  const filtered = useMemo(() => {
    if (!searchQuery) return sessions;
    const q = searchQuery.toLowerCase();
    return sessions.filter(
      (s) => s.title?.toLowerCase().includes(q) || s.id.toLowerCase().includes(q),
    );
  }, [sessions, searchQuery]);

  // Group sessions by workspace; apply sorting within groups.
  const { groups, ungrouped } = useMemo(() => {
    const g: { workspace: WorkspaceInfo | null; sessions: SessionInfo[] }[] = sortedWorkspaces.map(
      (ws) => ({
        workspace: ws,
        sessions: sortByUserOrder(
          filtered.filter((s) => sessionWorkspaceMap[s.id] === ws.id && !pinnedIds.has(s.id)),
        ),
      }),
    );
    const u = sortByUserOrder(filtered.filter((s) => !sessionWorkspaceMap[s.id] && !pinnedIds.has(s.id)));
    return { groups: g, ungrouped: u };
  }, [sortedWorkspaces, filtered, sessionWorkspaceMap, sortByUserOrder, pinnedIds]);

  // Pinned sessions (from filtered list, sorted separately)
  const pinnedSessions = useMemo(
    () => sortByUserOrder(filtered.filter((s) => pinnedIds.has(s.id))),
    [filtered, pinnedIds, sortByUserOrder],
  );

  const sessionPaneLayout = useMemo(
    () => getSessionPaneLayout({
      workspaceSectionCount: groups.length,
      ungroupedCount: ungrouped.length,
    }),
    [groups.length, ungrouped.length],
  );

  const flatVisibleIds = useMemo(() => {
    const ids: string[] = [];
    for (const s of pinnedSessions) ids.push(s.id);
    for (const { workspace, sessions: wsSessions } of groups) {
      if (workspace && collapsedIds.has(workspace.id)) continue;
      for (const s of wsSessions) ids.push(s.id);
    }
    for (const s of ungrouped) ids.push(s.id);
    return ids;
  }, [pinnedSessions, groups, ungrouped, collapsedIds]);

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

  useEffect(() => {
    if (!openMenu) return;

    const updateMenuPosition = () => {
      const anchor = menuAnchorRef.current;
      const menuElement = menuRef.current;
      if (!anchor || !anchor.isConnected || !menuElement) {
        closeOpenMenu();
        return;
      }

      const anchorRect = anchor.getBoundingClientRect();
      const menuRect = menuElement.getBoundingClientRect();
      const nextPosition = calculateFloatingMenuPosition({
        anchorRect: {
          top: anchorRect.top,
          right: anchorRect.right,
          bottom: anchorRect.bottom,
          left: anchorRect.left,
        },
        menuHeight: menuRect.height,
        menuWidth: menuRect.width,
        viewportHeight: window.innerHeight,
        viewportWidth: window.innerWidth,
      });

      setMenuPosition((prev) => {
        if (
          prev
          && prev.top === nextPosition.top
          && prev.left === nextPosition.left
          && prev.maxHeight === nextPosition.maxHeight
          && prev.direction === nextPosition.direction
        ) {
          return prev;
        }
        return nextPosition;
      });
    };

    updateMenuPosition();
    window.addEventListener('resize', updateMenuPosition);
    window.addEventListener('scroll', updateMenuPosition, true);

    return () => {
      window.removeEventListener('resize', updateMenuPosition);
      window.removeEventListener('scroll', updateMenuPosition, true);
    };
  }, [closeOpenMenu, openMenu]);

  // Session item click handler with shift/ctrl multi-select.
  const handleSessionClick = useCallback(
    (e: ReactMouseEvent, sessionId: string) => {
      if (e.shiftKey && lastClickedIdRef.current) {
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
        return;
      }

      if (e.ctrlKey || e.metaKey) {
        setSelectedIds((prev) => {
          const next = new Set(prev);
          if (next.has(sessionId)) next.delete(sessionId);
          else next.add(sessionId);
          return next;
        });
        lastClickedIdRef.current = sessionId;
        return;
      }

      if (selectedIds.size > 0) {
        setSelectedIds(new Set());
      }
      lastClickedIdRef.current = sessionId;
      onSelectSession(sessionId);
    },
    [flatVisibleIds, selectedIds.size, onSelectSession],
  );

  const handleBatchDelete = useCallback(() => {
    const ids = [...selectedIds];
    setSelectedIds(new Set());
    lastClickedIdRef.current = null;
    for (const id of ids) {
      onDeleteSession(id);
    }
  }, [selectedIds, onDeleteSession]);

  const startRename = useCallback((session: SessionInfo) => {
    setRenameValue(session.manual_title || session.title || '');
    setRenamingSessionId(session.id);
    setTimeout(() => renameInputRef.current?.focus(), 0);
  }, []);

  const commitRename = useCallback(() => {
    if (renamingSessionId) {
      const trimmed = renameValue.trim();
      onRenameSession(renamingSessionId, trimmed || null);
      setRenamingSessionId(null);
      setRenameValue('');
    }
  }, [renamingSessionId, renameValue, onRenameSession]);

  const cancelRename = useCallback(() => {
    setRenamingSessionId(null);
    setRenameValue('');
  }, []);

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

  const sessionById = useMemo(
    () => new Map(sessions.map((session) => [session.id, session])),
    [sessions],
  );
  const workspaceById = useMemo(
    () => new Map(workspaces.map((workspace) => [workspace.id, workspace])),
    [workspaces],
  );

  const openSession = openMenu?.kind === 'session' ? sessionById.get(openMenu.id) ?? null : null;
  const openWorkspace = openMenu?.kind === 'workspace' ? workspaceById.get(openMenu.id) ?? null : null;
  const openSessionWorkspace = openSession
    ? sortedWorkspaces.find((workspace) => workspace.id === sessionWorkspaceMap[openSession.id]) ?? null
    : null;
  const workspaceOptionsMaxHeight = calculateWorkspaceOptionsMaxHeight(menuPosition?.maxHeight ?? 0);

  // -- Toolbar action definitions --
  const wsToolbarActions: PanelToolbarAction[] = useMemo(() => [
    {
      icon: <FolderPlus size={13} />,
      title: 'New Workspace',
      onClick: () => setWsDialogOpen(true),
    },
  ], []);

  const sessionToolbarActions: PanelToolbarAction[] = useMemo(() => [
    {
      icon: <Plus size={13} />,
      title: 'New Session',
      onClick: onNewChat,
    },
  ], [onNewChat]);

  // -- Session item renderer --
  const renderSessionItem = (session: SessionInfo, groupSessionIds: string[], _isPinnedItem = false) => {
    const isStreaming = streamingSessionIds.has(session.id);
    const isActive = session.id === activeSessionId;
    const isSelected = selectedIds.has(session.id);
    const isDragging = draggedSessionId === session.id;
    const isPinned = pinnedIds.has(session.id);

    if (renamingSessionId === session.id) {
      return (
        <div
          key={session.id}
          data-session-id={session.id}
          className="session-item session-item--renaming"
        >
          <span className="session-item-spinner-placeholder" />
          <input
            ref={renameInputRef}
            className="session-rename-input"
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.stopPropagation();
                commitRename();
              } else if (e.key === 'Escape') {
                e.stopPropagation();
                cancelRename();
              }
            }}
            onBlur={commitRename}
            onClick={(e) => e.stopPropagation()}
          />
        </div>
      );
    }

    return (
      <div
        key={session.id}
        data-session-id={session.id}
        onDoubleClick={(e) => {
          e.stopPropagation();
          startRename(session);
        }}
      >
        <SessionItem
          session={{
            ...session,
            title: session.manual_title || session.title || 'Untitled Session',
          }}
          isActive={isActive}
          isStreaming={isStreaming}
          isPinned={isPinned}
          onPinToggle={(e) => {
            e.stopPropagation();
            togglePin(session.id);
          }}
          className={
            (isSelected ? 'session-item--selected ' : '') +
            (isDragging ? 'session-item--dragging' : '')
          }
          onClick={(e) => handleSessionClick(e, session.id)}
          onMouseDown={(e) => handleMouseDown(e, session.id, groupSessionIds)}
          onMouseMove={(e) => handleItemHover(e, session.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            toggleMenu({ kind: 'session', id: session.id }, e.currentTarget);
          }}
          actions={
            <button
              className="btn-session-action"
              onClick={(e) => {
                e.stopPropagation();
                toggleMenu({ kind: 'session', id: session.id }, e.currentTarget);
              }}
              title="Session actions"
            >
              <MoreHorizontal size={12} />
            </button>
          }
        />
      </div>
    );
  };

  // -- Floating context menu --
  const floatingMenu = (() => {
    if (!openMenu || typeof document === 'undefined') {
      return null;
    }

    const menuClassName = openMenu.kind === 'workspace'
      ? 'workspace-context-menu'
      : 'session-context-menu';

    let menuBody: ReactElement | null = null;
    if (openMenu.kind === 'workspace' && openWorkspace) {
      menuBody = (
        <>
          <button
            className="context-menu-item"
            onClick={() => {
              setEditingWorkspace(openWorkspace);
              closeOpenMenu();
            }}
          >
            <Pencil size={11} />
            Edit
          </button>
          <button
            className="context-menu-item"
            onClick={() => {
              revealItemInDir(openWorkspace.path);
              closeOpenMenu();
            }}
          >
            <FolderSymlink size={11} />
            Open in file manager
          </button>
          <hr className="context-menu-divider" />
          <button
            className="context-menu-item context-menu-item--danger"
            onClick={() => {
              onDeleteWorkspace(openWorkspace.id);
              closeOpenMenu();
            }}
          >
            <Trash2 size={11} />
            Delete workspace
          </button>
        </>
      );
    }

    if (openMenu.kind === 'session' && openSession) {
      menuBody = (
        <>
          {sortedWorkspaces.length > 0 && (
            <>
              <div className="context-menu-section">Move to workspace</div>
              <div
                className="context-menu-scroll-area"
                style={{ maxHeight: workspaceOptionsMaxHeight > 0 ? `${workspaceOptionsMaxHeight}px` : undefined }}
              >
                {sortedWorkspaces.map((workspace) => (
                  <button
                    key={workspace.id}
                    className="context-menu-item"
                    onClick={() => {
                      onAssignSession(workspace.id, openSession.id);
                      closeOpenMenu();
                    }}
                  >
                    <FolderOpen size={11} />
                    {workspace.name}
                    {openSessionWorkspace?.id === workspace.id && <span className="context-menu-check">*</span>}
                  </button>
                ))}
              </div>
              {openSessionWorkspace && (
                <button
                  className="context-menu-item"
                  onClick={() => {
                    onUnassignSession(openSession.id);
                    closeOpenMenu();
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
                  startRename(openSession);
                  closeOpenMenu();
                }}
              >
                <Pencil size={11} />
                Rename
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  onForkSession(openSession.id);
                  closeOpenMenu();
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
              onDeleteSession(openSession.id);
              closeOpenMenu();
            }}
          >
            <Trash2 size={11} />
            Delete session
          </button>
        </>
      );
    }

    if (!menuBody) {
      return null;
    }

    return createPortal(
      <div
        className={`${menuClassName} ${menuPosition?.direction === 'up' ? 'context-menu--up' : 'context-menu--down'}`}
        ref={menuRef}
        style={{
          top: menuPosition?.top ?? 0,
          left: menuPosition?.left ?? 0,
          maxHeight: menuPosition ? `${menuPosition.maxHeight}px` : undefined,
          visibility: menuPosition ? 'visible' : 'hidden',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {menuBody}
      </div>,
      document.body,
    );
  })();

  return (
    <>
      <div className="session-list">
        {pinnedSessions.length > 0 && (
          <div className="session-list-pinned">
            <PanelToolbar
              label="Pinned"
              sortValue={sessionSortField}
              onSortChange={setSessionSortField}
              collapsed={sectionCollapsed['pinned']}
              onCollapseToggle={() => toggleSectionCollapsed('pinned')}
            />
            {!sectionCollapsed['pinned'] && (
              <div className="session-list-pinned-items">
                {getPreviewList(pinnedSessions).map((session) => renderSessionItem(session, pinnedSessions.map((s) => s.id), true))}
              </div>
            )}
          </div>
        )}

        {sessionPaneLayout.showWorkspacePane && (
          <div className="session-list-workspace">
            <PanelToolbar
              label="Workspaces"
              sortValue={wsSortField}
              onSortChange={setWsSortField}
              collapsed={sectionCollapsed['workspace']}
              onCollapseToggle={() => toggleSectionCollapsed('workspace')}
              collapseToggle={{ allExpanded, onToggle: toggleAllExpanded }}
              actions={wsToolbarActions}
            />
            {!sectionCollapsed['workspace'] && (
              <div className="workspace-pane">
                {groups.map(({ workspace, sessions: originalWsSessions }) => {
                  if (!workspace) return null;
                  const isCollapsed = collapsedIds.has(workspace.id);
                  const wsSessions = getPreviewList(originalWsSessions);
                  return (
                    <div key={workspace.id} className="panel-group-section">
                      <div
                        className="panel-group-label"
                        onClick={() => toggleCollapsed(workspace.id)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          toggleMenu({ kind: 'workspace', id: workspace.id }, e.currentTarget);
                        }}
                        style={{ cursor: 'pointer' }}
                      >
                        {isCollapsed
                          ? <Folder size={11} className="panel-group-icon" />
                          : <FolderOpen size={11} className="panel-group-icon" />
                        }
                        <span className="panel-group-name" title={workspace.path}>
                          {workspace.name}
                        </span>
                        <button
                          className="panel-group-action-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            onNewChatInWorkspace(workspace.id);
                          }}
                          title="New session in this workspace"
                        >
                          <Plus size={12} />
                        </button>
                        <button
                          className="panel-group-action-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            toggleMenu({ kind: 'workspace', id: workspace.id }, e.currentTarget);
                          }}
                          title="Workspace options"
                        >
                          <MoreHorizontal size={12} />
                        </button>
                      </div>
                      {!isCollapsed && wsSessions.map((session) => renderSessionItem(session, originalWsSessions.map((item) => item.id)))}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {sessionPaneLayout.showGeneralPane && (
          <div className="session-list-general">
            <PanelToolbar
              label="Sessions"
              sortValue={sessionSortField}
              onSortChange={setSessionSortField}
              collapsed={sectionCollapsed['sessions']}
              onCollapseToggle={() => toggleSectionCollapsed('sessions')}
              actions={sessionToolbarActions}
            />
            {!sectionCollapsed['sessions'] && (
              <div className="session-pane">
                {getPreviewList(ungrouped).map((session) => renderSessionItem(session, ungrouped.map((item) => item.id)))}

                {filtered.length === 0 && (
                  <div className="session-empty">
                    {searchQuery ? 'No matching sessions' : 'No sessions yet'}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {filtered.length === 0 && !sessionPaneLayout.showGeneralPane && (
          <div className="session-empty">
            {searchQuery ? 'No matching sessions' : 'No sessions yet'}
          </div>
        )}

        {selectedIds.size > 0 && (
          <div className="batch-action-bar">
            <span className="batch-action-count">{selectedIds.size} selected</span>
            <Button
              variant="icon"
              size="sm"
              onClick={handleBatchDelete}
              title="Delete"
              className="batch-action-delete-btn"
            >
              <Trash2 size={14} />
            </Button>
          </div>
        )}
      </div>

      {floatingMenu}

      {wsDialogOpen && (
        <WorkspaceDialog
          onConfirm={(name, path) => {
            onCreateWorkspace(name, path);
            setWsDialogOpen(false);
          }}
          onClose={() => setWsDialogOpen(false)}
        />
      )}

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
