import { useState, useRef, useEffect, useMemo } from 'react';
import { X, Plus, FolderOpen, MoreHorizontal, Pencil, Trash2, ChevronRight, ChevronDown, MessageSquare, Zap, Puzzle, BookOpen, Database, Bot, Loader2, CheckCircle2, AlertCircle } from 'lucide-react';
import type { SessionInfo, WorkspaceInfo, SkillInfo, KnowledgeCollectionInfo } from '../types';
import type { ImportStatus } from '../hooks/useSkills';
import type { KbIngestStatus, KbBatchProgress } from '../hooks/useKnowledge';
import { WorkspaceDialog } from './WorkspaceDialog';
import './Sidebar.css';

export type ViewType = 'chat' | 'automation' | 'skills' | 'knowledge' | 'agents';

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
  // Knowledge props
  knowledgeCollections: KnowledgeCollectionInfo[];
  selectedKbCollection: string | null;
  onSelectKbCollection: (name: string) => void;
  onCreateKbCollection: (name: string, description: string) => void;
  // Knowledge ingest status (shown below collection list)
  kbIngestStatus: KbIngestStatus;
  kbBatchProgress: KbBatchProgress | null;
  kbIngestError: string | null;
  onClearKbIngestStatus: () => void;
  onCancelKbIngest: () => void;
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
  // Agent props
  agents: { id: string; name: string; description: string; mode: string; trust_tier: string; is_overridden: boolean }[];
  activeAgentId: string | null;
  onSelectAgent: (id: string) => void;
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
  knowledgeCollections,
  selectedKbCollection,
  onSelectKbCollection,
  onCreateKbCollection,
  importStatus,
  importError,
  kbIngestStatus,
  kbBatchProgress,
  kbIngestError,
  onClearKbIngestStatus,
  onCancelKbIngest,
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
  agents,
  activeAgentId,
  onSelectAgent,
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [skillSearchQuery, setSkillSearchQuery] = useState('');
  const [kbSearchQuery, setKbSearchQuery] = useState('');
  const [showNewKbCollection, setShowNewKbCollection] = useState(false);
  const [newKbCollName, setNewKbCollName] = useState('');
  const [newKbCollDesc, setNewKbCollDesc] = useState('');
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
      {/* VS Code-style vertical icon bar */}
      <div className="sidebar-icon-bar">
        <button
          className={`sidebar-nav-btn ${activeView === 'chat' ? 'active' : ''}`}
          onClick={() => onSelectView('chat')}
          title="Sessions"
        >
          <MessageSquare size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'automation' ? 'active' : ''}`}
          onClick={() => onSelectView('automation')}
          title="Automation"
        >
          <Zap size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'skills' ? 'active' : ''}`}
          onClick={() => onSelectView('skills')}
          title="Skills"
        >
          <Puzzle size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'knowledge' ? 'active' : ''}`}
          onClick={() => onSelectView('knowledge')}
          title="Knowledge"
        >
          <BookOpen size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'agents' ? 'active' : ''}`}
          onClick={() => onSelectView('agents')}
          title="Agents"
        >
          <Bot size={20} />
        </button>
      </div>

      {/* Sidebar panel content */}
      <div className="sidebar-panel">

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

      {/* Agents view — agent list */}
      {activeView === 'agents' && (
        <div className="skill-sidebar-list">
          {(() => {
            if (agents.length === 0) {
              return (
                <div className="session-empty">No agents registered</div>
              );
            }
            const tiers = ['BuiltIn', 'UserDefined', 'Dynamic'] as const;
            const tierLabels: Record<string, string> = {
              BuiltIn: 'Built-in',
              UserDefined: 'User-Defined',
              Dynamic: 'Dynamic',
            };
            return tiers.map((tier) => {
              const tierAgents = agents.filter((a) => a.trust_tier === tier);
              if (tierAgents.length === 0) return null;
              return (
                <div key={tier}>
                  <div className="workspace-label workspace-label--general">
                    <span className="workspace-name">{tierLabels[tier]} ({tierAgents.length})</span>
                  </div>
                  {tierAgents.map((agent) => (
                    <div
                      key={agent.id}
                      className={`skill-sidebar-item ${activeAgentId === agent.id ? 'active' : ''}`}
                      onClick={() => onSelectAgent(agent.id)}
                    >
                      <div className="skill-sidebar-item-header">
                        <Bot size={14} className="skill-sidebar-item-icon" />
                        <span className="skill-sidebar-item-name">{agent.name}</span>
                        {agent.is_overridden && (
                          <span className="skill-sidebar-item-badge" style={{ color: '#ffc107' }}>OVR</span>
                        )}
                      </div>
                      <p className="skill-sidebar-item-desc">{agent.description}</p>
                    </div>
                  ))}
                </div>
              );
            });
          })()}
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

      {/* Knowledge view — collection list */}
      {activeView === 'knowledge' && (
        <>
          <div className="sidebar-header">
            <div className="sidebar-search">
              <input
                type="text"
                placeholder="Search collections..."
                value={kbSearchQuery}
                onChange={(e) => setKbSearchQuery(e.target.value)}
                className="search-input"
              />
            </div>
            <div className="sidebar-header-actions">
              <button
                className="btn-new-chat"
                onClick={() => setShowNewKbCollection(true)}
                title="New Collection"
              >
                <Plus size={16} />
              </button>
            </div>
          </div>
          <div className="skill-sidebar-list">
            {(() => {
              const q = kbSearchQuery.toLowerCase();
              const filtered = q
                ? knowledgeCollections.filter(c =>
                    c.name.toLowerCase().includes(q) ||
                    c.description.toLowerCase().includes(q)
                  )
                : knowledgeCollections;

              if (filtered.length === 0) {
                return (
                  <div className="session-empty">
                    {kbSearchQuery ? 'No matching collections' : 'No collections yet'}
                  </div>
                );
              }

              return filtered.map((coll) => {
                const sizeLabel = coll.total_bytes > 0
                  ? coll.total_bytes >= 1048576
                    ? `${(coll.total_bytes / 1048576).toFixed(1)} MB`
                    : coll.total_bytes >= 1024
                      ? `${(coll.total_bytes / 1024).toFixed(1)} KB`
                      : `${coll.total_bytes} B`
                  : null;
                const chunkLabel = coll.chunk_count >= 1000
                  ? `${(coll.chunk_count / 1000).toFixed(1)}K`
                  : String(coll.chunk_count);
                return (
                  <div
                    key={coll.name}
                    className={`skill-sidebar-item ${selectedKbCollection === coll.name ? 'active' : ''}`}
                    onClick={() => onSelectKbCollection(coll.name)}
                  >
                    <div className="skill-sidebar-item-header">
                      <Database size={14} className="skill-sidebar-item-icon" />
                      <span className="skill-sidebar-item-name">{coll.name}</span>
                      <span className="skill-sidebar-item-badge">{coll.entry_count}</span>
                    </div>
                    <p className="skill-sidebar-item-desc">
                      {coll.entry_count > 0
                        ? `${coll.entry_count} entries · ${chunkLabel} chunks${sizeLabel ? ` · ${sizeLabel}` : ''}`
                        : coll.description || 'No description'}
                    </p>
                  </div>
                );
              });
            })()}
          </div>

          {/* Inline new collection form */}
          {showNewKbCollection && (
            <div className="kb-new-collection-form">
              <div className="kb-new-collection-form-header">
                <span className="kb-new-collection-form-title">New Collection</span>
                <button
                  className="kb-new-collection-form-close"
                  onClick={() => setShowNewKbCollection(false)}
                  title="Close"
                >
                  <X size={12} />
                </button>
              </div>
              <input
                className="search-input"
                placeholder="Collection name"
                value={newKbCollName}
                onChange={e => setNewKbCollName(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter' && newKbCollName.trim()) {
                    onCreateKbCollection(newKbCollName.trim(), newKbCollDesc.trim());
                    setNewKbCollName('');
                    setNewKbCollDesc('');
                    setShowNewKbCollection(false);
                  }
                }}
                autoFocus
              />
              <input
                className="search-input"
                placeholder="Description (optional)"
                value={newKbCollDesc}
                onChange={e => setNewKbCollDesc(e.target.value)}
              />
              <button
                className="kb-new-collection-create-btn"
                disabled={!newKbCollName.trim()}
                onClick={() => {
                  if (newKbCollName.trim()) {
                    onCreateKbCollection(newKbCollName.trim(), newKbCollDesc.trim());
                    setNewKbCollName('');
                    setNewKbCollDesc('');
                    setShowNewKbCollection(false);
                  }
                }}
              >
                Create
              </button>
            </div>
          )}

          {/* Knowledge ingest status bar */}
          {kbIngestStatus !== 'idle' && (
            <div className={`skill-import-status skill-import-status--${kbIngestStatus === 'ingesting' ? 'importing' : kbIngestStatus}`}>
              <div className="skill-import-status-row">
                {kbIngestStatus === 'ingesting' && (
                  <>
                    <Loader2 size={14} className="skill-import-status-spinner" />
                    <span className="skill-import-status-msg">
                      {kbBatchProgress
                        ? `Importing ${kbBatchProgress.current}/${kbBatchProgress.total}…`
                        : 'Importing…'}
                    </span>
                  </>
                )}
                {kbIngestStatus === 'success' && (
                  <>
                    <CheckCircle2 size={14} />
                    <span className="skill-import-status-msg">
                      {kbBatchProgress
                        ? `${kbBatchProgress.total} file${kbBatchProgress.total > 1 ? 's' : ''} imported`
                        : 'Import complete'}
                    </span>
                  </>
                )}
                {kbIngestStatus === 'error' && (
                  <>
                    <AlertCircle size={14} />
                    <span className="skill-import-status-msg">{kbIngestError || 'Import failed'}</span>
                  </>
                )}
                <div className="skill-import-status-actions">
                  {kbIngestStatus === 'ingesting' && (
                    <button className="skill-import-status-dismiss" onClick={onCancelKbIngest} title="Cancel">
                      <X size={12} />
                    </button>
                  )}
                  {kbIngestStatus !== 'ingesting' && (
                    <button className="skill-import-status-dismiss" onClick={onClearKbIngestStatus} title="Dismiss">
                      <X size={12} />
                    </button>
                  )}
                </div>
              </div>
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
      </div>{/* end sidebar-panel */}
    </aside>
  );
}
