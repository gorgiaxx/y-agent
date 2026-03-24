import { useState } from 'react';
import {
  MessageSquare,
  Zap,
  Puzzle,
  BookOpen,
  Bot,
  Settings,
} from 'lucide-react';
import type { SessionInfo, WorkspaceInfo, SkillInfo, KnowledgeCollectionInfo } from '../types';
import type { ImportStatus } from '../hooks/useSkills';
import type { KbIngestStatus, KbBatchProgress } from '../hooks/useKnowledge';
import { ChatSidebarPanel } from './chat-panel/ChatSidebarPanel';
import { SkillsSidebarPanel } from './skills/SkillsSidebarPanel';
import { KnowledgeSidebarPanel } from './knowledge/KnowledgeSidebarPanel';
import { AgentsSidebarPanel } from './agents/AgentsSidebarPanel';
import { SettingsSidebarNav } from './settings/SettingsSidebarNav';
import './Sidebar.css';

export type ViewType = 'chat' | 'automation' | 'skills' | 'knowledge' | 'agents' | 'settings';

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
  activeSettingsTab: string | null;
  onSelectSettingsTab: (tab: string) => void;
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
  activeSettingsTab,
  onSelectSettingsTab,
}: SidebarProps) {
  const [panelCollapsed, setPanelCollapsed] = useState(false);

  // Handle icon bar clicks: toggle panel collapse when clicking the active view,
  // switch view and expand panel when clicking a different view.
  const handleIconClick = (view: ViewType) => {
    if (view === activeView && !panelCollapsed) {
      // Same view is active and panel is open -- collapse
      setPanelCollapsed(true);
    } else {
      // Different view or panel is collapsed -- switch and expand
      onSelectView(view);
      setPanelCollapsed(false);
    }
  };

  return (
    <aside className="sidebar">
      {/* VS Code-style vertical icon bar */}
      <div className="sidebar-icon-bar">
        <button
          className={`sidebar-nav-btn ${activeView === 'chat' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('chat')}
          title="Sessions"
        >
          <MessageSquare size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'automation' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('automation')}
          title="Automation"
        >
          <Zap size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'skills' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('skills')}
          title="Skills"
        >
          <Puzzle size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'knowledge' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('knowledge')}
          title="Knowledge"
        >
          <BookOpen size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${activeView === 'agents' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('agents')}
          title="Agents"
        >
          <Bot size={20} />
        </button>
        <div className="sidebar-icon-spacer" />
        <button
          className={`sidebar-nav-btn ${activeView === 'settings' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('settings')}
          title="Settings"
          id="btn-settings"
        >
          <Settings size={20} />
        </button>
      </div>

      {/* Sidebar panel content */}
      <div className={`sidebar-panel ${panelCollapsed ? 'sidebar-panel--collapsed' : ''}`}>

      {/* Sessions content (only when chat view is active) */}
      {activeView === 'chat' && (
        <ChatSidebarPanel
          sessions={sessions}
          activeSessionId={activeSessionId}
          streamingSessionIds={streamingSessionIds}
          workspaces={workspaces}
          sessionWorkspaceMap={sessionWorkspaceMap}
          onSelectSession={onSelectSession}
          onNewChat={onNewChat}
          onNewChatInWorkspace={onNewChatInWorkspace}
          onDeleteSession={onDeleteSession}
          onCreateWorkspace={onCreateWorkspace}
          onUpdateWorkspace={onUpdateWorkspace}
          onDeleteWorkspace={onDeleteWorkspace}
          onAssignSession={onAssignSession}
          onUnassignSession={onUnassignSession}
        />
      )}

      {/* Automation placeholder */}
      {activeView === 'automation' && (
        <div className="sidebar-placeholder">
          <Zap size={32} className="sidebar-placeholder-icon" />
          <p className="sidebar-placeholder-text">Coming soon</p>
        </div>
      )}

      {/* Agents view -- agent list */}
      {activeView === 'agents' && (
        <AgentsSidebarPanel
          agents={agents}
          activeAgentId={activeAgentId}
          onSelectAgent={onSelectAgent}
        />
      )}

      {/* Settings view -- category list */}
      {activeView === 'settings' && (
        <SettingsSidebarNav
          activeTab={activeSettingsTab}
          onSelectTab={onSelectSettingsTab}
        />
      )}

      {/* Skills view -- skill list */}
      {activeView === 'skills' && (
        <SkillsSidebarPanel
          skills={skills}
          activeSkillName={activeSkillName}
          importStatus={importStatus}
          importError={importError}
          onSelectSkill={onSelectSkill}
          onImportClick={onImportClick}
          onClearImportStatus={onClearImportStatus}
        />
      )}

      {/* Knowledge view -- collection list */}
      {activeView === 'knowledge' && (
        <KnowledgeSidebarPanel
          collections={knowledgeCollections}
          selectedCollection={selectedKbCollection}
          onSelectCollection={onSelectKbCollection}
          onCreateCollection={onCreateKbCollection}
          kbIngestStatus={kbIngestStatus}
          kbBatchProgress={kbBatchProgress}
          kbIngestError={kbIngestError}
          onClearKbIngestStatus={onClearKbIngestStatus}
          onCancelKbIngest={onCancelKbIngest}
        />
      )}

      </div>{/* end sidebar-panel */}
    </aside>
  );
}
