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
import type { WorkflowInfo, ScheduleInfo } from '../hooks/useAutomation';
import { ChatSidebarPanel } from './chat-panel/ChatSidebarPanel';
import { SkillsSidebarPanel } from './skills/SkillsSidebarPanel';
import { KnowledgeSidebarPanel } from './knowledge/KnowledgeSidebarPanel';
import { AutomationSidebarPanel } from './automation/AutomationSidebarPanel';
import { SettingsSidebarNav } from './settings/SettingsSidebarNav';
import './Sidebar.css';

export type ViewType = 'chat' | 'automation' | 'skills' | 'knowledge' | 'agents' | 'settings';

// ---------------------------------------------------------------------------
// Grouped prop interfaces -- reduces 38 flat props to 5 domain groups
// ---------------------------------------------------------------------------

/** Chat/Session domain props. */
export interface ChatSidebarProps {
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
}

/** Skills domain props. */
export interface SkillsSidebarPropsGroup {
  skills: SkillInfo[];
  activeSkillName: string | null;
  importStatus: ImportStatus;
  importError: string | null;
  onSelectSkill: (name: string) => void;
  onImportClick: () => void;
  onClearImportStatus: () => void;
}

/** Knowledge domain props. */
export interface KnowledgeSidebarPropsGroup {
  collections: KnowledgeCollectionInfo[];
  selectedCollection: string | null;
  onSelectCollection: (name: string) => void;
  onCreateCollection: (name: string, description: string) => void;
  ingestStatus: KbIngestStatus;
  batchProgress: KbBatchProgress | null;
  ingestError: string | null;
  onClearIngestStatus: () => void;
  onCancelIngest: () => void;
}

/** Automation domain props. */
export interface AutomationSidebarPropsGroup {
  workflows: WorkflowInfo[];
  schedules: ScheduleInfo[];
  selectedType: 'workflow' | 'schedule' | null;
  selectedId: string | null;
  onSelectWorkflow: (id: string) => void;
  onSelectSchedule: (id: string) => void;
  onCreateWorkflow: () => void;
  onCreateSchedule: () => void;
}

/** Navigation / settings props. */
export interface NavSidebarPropsGroup {
  activeView: ViewType;
  onSelectView: (view: ViewType) => void;
  activeSettingsTab: string | null;
  onSelectSettingsTab: (tab: string) => void;
}

// ---------------------------------------------------------------------------
// Sidebar component
// ---------------------------------------------------------------------------

interface SidebarProps {
  chat: ChatSidebarProps;
  skills: SkillsSidebarPropsGroup;
  knowledge: KnowledgeSidebarPropsGroup;
  automation: AutomationSidebarPropsGroup;
  nav: NavSidebarPropsGroup;
}

export function Sidebar({ chat, skills, knowledge, automation, nav }: SidebarProps) {
  const [panelCollapsed, setPanelCollapsed] = useState(false);

  // Handle icon bar clicks: toggle panel collapse when clicking the active view,
  // switch view and expand panel when clicking a different view.
  const handleIconClick = (view: ViewType) => {
    if (view === 'settings') {
      // Settings panel is always visible -- no collapse toggle
      nav.onSelectView(view);
      setPanelCollapsed(false);
      return;
    }
    if (view === 'agents') {
      nav.onSelectView(view);
      setPanelCollapsed(false);
      return;
    }
    if (view === nav.activeView && !panelCollapsed) {
      // Same view is active and panel is open -- collapse
      setPanelCollapsed(true);
    } else {
      // Different view or panel is collapsed -- switch and expand
      nav.onSelectView(view);
      setPanelCollapsed(false);
    }
  };

  return (
    <aside className="sidebar">
      {/* VS Code-style vertical icon bar */}
      <div className="sidebar-icon-bar">
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'chat' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('chat')}
          title="Sessions"
        >
          <MessageSquare size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'automation' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('automation')}
          title="Automation"
        >
          <Zap size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'skills' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('skills')}
          title="Skills"
        >
          <Puzzle size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'knowledge' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('knowledge')}
          title="Knowledge"
        >
          <BookOpen size={20} />
        </button>
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'agents' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('agents')}
          title="Agents"
        >
          <Bot size={20} />
        </button>
        <div className="sidebar-icon-spacer" />
        <button
          className={`sidebar-nav-btn ${nav.activeView === 'settings' && !panelCollapsed ? 'active' : ''}`}
          onClick={() => handleIconClick('settings')}
          title="Settings"
          id="btn-settings"
        >
          <Settings size={20} />
        </button>
      </div>

      {/* Sidebar panel content */}
      <div
        className={`sidebar-panel ${panelCollapsed || nav.activeView === 'agents' ? 'sidebar-panel--collapsed' : ''} ${nav.activeView === 'settings' ? 'sidebar-panel--settings' : ''}`}
      >

      {/* Sessions content (only when chat view is active) */}
      {nav.activeView === 'chat' && (
        <ChatSidebarPanel
          sessions={chat.sessions}
          activeSessionId={chat.activeSessionId}
          streamingSessionIds={chat.streamingSessionIds}
          workspaces={chat.workspaces}
          sessionWorkspaceMap={chat.sessionWorkspaceMap}
          onSelectSession={chat.onSelectSession}
          onNewChat={chat.onNewChat}
          onNewChatInWorkspace={chat.onNewChatInWorkspace}
          onDeleteSession={chat.onDeleteSession}
          onForkSession={chat.onForkSession}
          onRenameSession={chat.onRenameSession}
          onCreateWorkspace={chat.onCreateWorkspace}
          onUpdateWorkspace={chat.onUpdateWorkspace}
          onDeleteWorkspace={chat.onDeleteWorkspace}
          onAssignSession={chat.onAssignSession}
          onUnassignSession={chat.onUnassignSession}
        />
      )}

      {/* Automation sidebar */}
      {nav.activeView === 'automation' && (
        <AutomationSidebarPanel
          workflows={automation.workflows}
          schedules={automation.schedules}
          selectedType={automation.selectedType}
          selectedId={automation.selectedId}
          onSelectWorkflow={automation.onSelectWorkflow}
          onSelectSchedule={automation.onSelectSchedule}
          onCreateWorkflow={automation.onCreateWorkflow}
          onCreateSchedule={automation.onCreateSchedule}
        />
      )}

      {/* Settings view -- category list */}
      {nav.activeView === 'settings' && (
        <SettingsSidebarNav
          activeTab={nav.activeSettingsTab}
          onSelectTab={nav.onSelectSettingsTab}
        />
      )}

      {/* Skills view -- skill list */}
      {nav.activeView === 'skills' && (
        <SkillsSidebarPanel
          skills={skills.skills}
          activeSkillName={skills.activeSkillName}
          importStatus={skills.importStatus}
          importError={skills.importError}
          onSelectSkill={skills.onSelectSkill}
          onImportClick={skills.onImportClick}
          onClearImportStatus={skills.onClearImportStatus}
        />
      )}

      {/* Knowledge view -- collection list */}
      {nav.activeView === 'knowledge' && (
        <KnowledgeSidebarPanel
          collections={knowledge.collections}
          selectedCollection={knowledge.selectedCollection}
          onSelectCollection={knowledge.onSelectCollection}
          onCreateCollection={knowledge.onCreateCollection}
          kbIngestStatus={knowledge.ingestStatus}
          kbBatchProgress={knowledge.batchProgress}
          kbIngestError={knowledge.ingestError}
          onClearKbIngestStatus={knowledge.onClearIngestStatus}
          onCancelKbIngest={knowledge.onCancelIngest}
        />
      )}

      </div>{/* end sidebar-panel */}
    </aside>
  );
}
