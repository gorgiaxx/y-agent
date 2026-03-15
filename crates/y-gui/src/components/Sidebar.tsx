import { useState, useRef, useEffect, useMemo } from 'react';
import { X, Plus, FolderOpen, MoreHorizontal, Pencil, Trash2, ChevronRight, ChevronDown, MessageSquare, Zap, Puzzle, Loader2, CheckCircle2, AlertCircle } from 'lucide-react';
import type { SessionInfo, WorkspaceInfo, SkillInfo } from '../types';
import type { ImportStatus } from '../hooks/useSkills';
import { WorkspaceDialog } from './WorkspaceDialog';
import './Sidebar.css';

export type ViewType = 'chat' | 'automation' | 'skills';

interface SidebarProps {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  streamingSessionIds: Set<string>;
  workspaces: WorkspaceInfo[];
  sessionWorkspaceMap: Record<string, string>;
  activeView: ViewType;
  skills: SkillInfo[];
  activeSkillName: string | null;
  importStatus: ImportStatus;
  importError: string | null;
  onSelectView: (view: ViewType) => void;
  onSelectSession: (id: string) => void;
  onSelectSkill: (name: string) => void;
  onImportClick: () => void;
  onClearImportStatus: () => void;
  onNewChat: () => void;
  onNewChatInWorkspace: (workspaceId: string) => void;
  onDeleteSession: (id: string) => void;
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

export function Sidebar({
  sessions,
  activeSessionId,
  streamingSessionIds,
  workspaces,
  sessionWorkspaceMap,
  activeView,
  skills,
  activeSkillName,
  importStatus,
  importError,
  onSelectView,
  onSelectSession,
  onSelectSkill,
  onImportClick,
  onClearImportStatus,
  onNewChat,
  onNewChatInWorkspace,
  onDeleteSession,
  onCreateWorkspace,
  onUpdateWorkspace,
  onDeleteWorkspace,
  onAssignSession,
  onUnassignSession,
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [skillSearchQuery, setSkillSearchQuery] = useState('');
  const [wsDialogOpen, setWsDialogOpen] = useState(false);
  const [editingWorkspace, setEditingWorkspace] = useState<WorkspaceInfo | null>(null);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [openSessionMenuId, setOpenSessionMenuId] = useState<string | null>(null);
  const [importStatusExpanded, setImportStatusExpanded] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // Sorted workspaces by name (alphabetically).
  const sortedWorkspaces = useMemo(
    () => [...workspaces].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
    ),
    [workspaces],
  );

  // Default: only the first workspace (alphabetically) is expanded.
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(
    () => new Set(sortedWorkspaces.slice(1).map((w) => w.id)),
  );

  const toggleCollapsed = (id: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Close menus when clicking outside.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpenMenuId(null);
        setOpenSessionMenuId(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  const filtered = useMemo(() => {
    if (!searchQuery) return sessions;
    const q = searchQuery.toLowerCase();
    return sessions.filter(
      (s) => s.title?.toLowerCase().includes(q) || s.id.toLowerCase().includes(q),
    );
  }, [sessions, searchQuery]);

  // Group sessions by workspace, sorted alphabetically.
  const { groups, ungrouped } = useMemo(() => {
    const g: { workspace: WorkspaceInfo | null; sessions: SessionInfo[] }[] = sortedWorkspaces.map(
      (ws) => ({
        workspace: ws,
        sessions: filtered.filter((s) => sessionWorkspaceMap[s.id] === ws.id),
      }),
    );
    const u = filtered.filter((s) => !sessionWorkspaceMap[s.id]);
    return { groups: g, ungrouped: u };
  }, [sortedWorkspaces, filtered, sessionWorkspaceMap]);

  const renderSessionItem = (session: SessionInfo) => {
    const isStreaming = streamingSessionIds.has(session.id);
    const isActive = session.id === activeSessionId;
    const timeLabel = formatRelativeTime(session.updated_at, isStreaming);
    const assignedWs = workspaces.find((w) => w.id === sessionWorkspaceMap[session.id]);

    return (
      <div
        key={session.id}
        className={`session-item ${isActive ? 'active' : ''} ${isStreaming ? 'streaming' : ''}`}
        onClick={() => onSelectSession(session.id)}
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
    <aside className="sidebar">
      {/* Navigation tabs */}
      <div className="sidebar-nav">
        <button
          className={`sidebar-nav-btn ${activeView === 'chat' ? 'active' : ''}`}
          onClick={() => onSelectView('chat')}
          title="Sessions"
        >
          <MessageSquare size={16} />
          <span className="sidebar-nav-label">Sessions</span>
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'automation' ? 'active' : ''}`}
          onClick={() => onSelectView('automation')}
          title="Automation"
        >
          <Zap size={16} />
          <span className="sidebar-nav-label">Automation</span>
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'skills' ? 'active' : ''}`}
          onClick={() => onSelectView('skills')}
          title="Skills"
        >
          <Puzzle size={16} />
          <span className="sidebar-nav-label">Skills</span>
        </button>
      </div>

      {/* Sessions content (only when chat view is active) */}
      {activeView === 'chat' && (
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
            {groups.map(({ workspace, sessions: wsSessions }) => {
              if (!workspace) return null;
              const isCollapsed = collapsedIds.has(workspace.id);
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
                  {!isCollapsed && wsSessions.map(renderSessionItem)}
                </div>
              );
            })}

            {/* Ungrouped sessions */}
            {ungrouped.length > 0 && (
              <div className="workspace-section">
                {workspaces.length > 0 && (
                  <div className="workspace-label workspace-label--general">
                    <span className="workspace-name">General</span>
                  </div>
                )}
                {ungrouped.map(renderSessionItem)}
              </div>
            )}

            {filtered.length === 0 && (
              <div className="session-empty">
                {searchQuery ? 'No matching sessions' : 'No sessions yet'}
              </div>
            )}
          </div>
        </>
      )}

      {/* Automation placeholder */}
      {activeView === 'automation' && (
        <div className="sidebar-placeholder">
          <Zap size={32} className="sidebar-placeholder-icon" />
          <p className="sidebar-placeholder-text">Coming soon</p>
        </div>
      )}

      {/* Skills view — skill list */}
      {activeView === 'skills' && (
        <>
          <div className="sidebar-header">
            <div className="sidebar-search">
              <input
                type="text"
                placeholder="Search skills..."
                value={skillSearchQuery}
                onChange={(e) => setSkillSearchQuery(e.target.value)}
                className="search-input"
              />
            </div>
            <div className="sidebar-header-actions">
              <button className="btn-new-chat" onClick={onImportClick} title="Import Skill">
                <Plus size={16} />
              </button>
            </div>
          </div>
          <div className="skill-sidebar-list">
            {(() => {
              const q = skillSearchQuery.toLowerCase();
              const filtered = q
                ? skills.filter(
                    (s) =>
                      s.name.toLowerCase().includes(q) ||
                      s.tags.some((t) => t.toLowerCase().includes(q)) ||
                      s.description.toLowerCase().includes(q),
                  )
                : skills;

              if (filtered.length === 0) {
                return (
                  <div className="session-empty">
                    {skillSearchQuery ? 'No matching skills' : 'No skills installed'}
                  </div>
                );
              }

              return filtered.map((skill) => (
                <div
                  key={skill.name}
                  className={`skill-sidebar-item ${activeSkillName === skill.name ? 'active' : ''} ${!skill.enabled ? 'skill-sidebar-item--disabled' : ''}`}
                  onClick={() => onSelectSkill(skill.name)}
                >
                  <div className="skill-sidebar-item-header">
                    <Puzzle size={14} className="skill-sidebar-item-icon" />
                    <span className="skill-sidebar-item-name">{skill.name}</span>
                    {!skill.enabled && (
                      <span className="skill-sidebar-item-badge">OFF</span>
                    )}
                  </div>
                  <p className="skill-sidebar-item-desc">{skill.description}</p>
                  {skill.tags.length > 0 && (
                    <div className="skill-sidebar-item-tags">
                      {skill.tags.slice(0, 3).map((tag) => (
                        <span key={tag} className="skill-sidebar-item-tag">{tag}</span>
                      ))}
                    </div>
                  )}
                </div>
              ));
            })()}
          </div>

          {/* Import status bar */}
          {importStatus !== 'idle' && (
            <div className={`skill-import-status skill-import-status--${importStatus} ${importStatusExpanded ? 'skill-import-status--expanded' : ''}`}>
              <div className="skill-import-status-row">
                {importStatus === 'importing' && (
                  <>
                    <Loader2 size={14} className="skill-import-status-spinner" />
                    <span className="skill-import-status-msg">Importing skill…</span>
                  </>
                )}
                {importStatus === 'success' && (
                  <>
                    <CheckCircle2 size={14} />
                    <span className="skill-import-status-msg">Skill imported</span>
                  </>
                )}
                {importStatus === 'error' && (
                  <>
                    <AlertCircle size={14} />
                    <span className="skill-import-status-msg">{importError || 'Import failed'}</span>
                  </>
                )}
                <div className="skill-import-status-actions">
                  {(importError || importStatus === 'importing') && (
                    <button
                      className="skill-import-status-toggle"
                      onClick={() => setImportStatusExpanded(!importStatusExpanded)}
                      title={importStatusExpanded ? 'Collapse' : 'Expand'}
                    >
                      {importStatusExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                    </button>
                  )}
                  {importStatus === 'error' && (
                    <button className="skill-import-status-dismiss" onClick={() => { onClearImportStatus(); setImportStatusExpanded(false); }} title="Dismiss">
                      <X size={12} />
                    </button>
                  )}
                </div>
              </div>
              {importStatusExpanded && (importError || importStatus === 'importing') && (
                <div className="skill-import-status-detail">
                  <pre className="skill-import-status-pre">{importError || 'Processing…'}</pre>
                </div>
              )}
            </div>
          )}
        </>
      )}

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
    </aside>
  );
}
