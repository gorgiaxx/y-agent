import { useRef } from 'react';
import {
  Zap,
  Puzzle,
  BookOpen,
  Bot,
  Settings as SettingsIcon,
  Plus,
  Search,
  ArrowLeft,
} from 'lucide-react';
import type { SessionInfo, WorkspaceInfo } from '../types';
import { ChatSidebarPanel } from './chat-panel/ChatSidebarPanel';
import { NavSidebar, NavItem, NavDivider } from './common/NavSidebar';
import { SettingsSidebarNav } from './settings/SettingsSidebarNav';
import { AgentEditorSidebarNav } from './agents/AgentEditorSidebarNav';
import { AgentStudioSidebarNav } from './agents/AgentStudioSidebarNav';
import { SkillsSidebarContent } from './skills/SkillsSidebarContent';
import { KnowledgeSidebarContent } from './knowledge/KnowledgeSidebarContent';
import { AutomationSidebarContent } from './automation/AutomationSidebarContent';
import { BackgroundTasksSidebarContent } from './background-tasks/BackgroundTasksSidebarContent';
import { BackgroundTasksSidebarNav } from './background-tasks/BackgroundTasksPanel';
import { useSidebarSearch } from '../hooks/useSidebarSearch';
import type { EditorTab, EditorSurface } from './agents/types';
import './Sidebar.css';

export type ViewType = 'chat' | 'automation' | 'skills' | 'knowledge' | 'agents' | 'settings';

// ---------------------------------------------------------------------------
// Grouped prop interfaces
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

/** Navigation props. */
export interface NavSidebarPropsGroup {
  activeView: ViewType;
  onSelectView: (view: ViewType) => void;
  activeSettingsTab: string | null;
  onSelectSettingsTab: (tab: string) => void;
  agentEditing: boolean;
  agentEditorTab: EditorTab;
  agentEditorSurface: EditorSurface;
  onAgentEditorTabChange: (tab: EditorTab) => void;
  onAgentEditorSurfaceChange: (surface: EditorSurface) => void;
  onAgentEditorBack: () => void;
  sessionPromptEditing: boolean;
  onSessionPromptBack: () => void;
  backgroundTasksOpen: boolean;
  onCloseBackgroundTasks: () => void;
}

/** Agent studio props (shown when an agent is selected but not being edited). */
export interface AgentStudioSidebarProps {
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

interface SidebarProps {
  chat: ChatSidebarProps;
  nav: NavSidebarPropsGroup;
  agentStudio?: AgentStudioSidebarProps | null;
}

export function Sidebar({ chat, nav, agentStudio }: SidebarProps) {
  const { searchQuery, setSearchQuery, searchOpen, setSearchOpen, searchInputRef, closeSearch } = useSidebarSearch();
  const toggleRef = useRef<HTMLButtonElement>(null);

  const goTo = (view: ViewType) => nav.onSelectView(view);

  if (nav.backgroundTasksOpen) {
    return (
      <BackgroundTasksSidebarNav onBack={nav.onCloseBackgroundTasks}>
        <BackgroundTasksSidebarContent />
      </BackgroundTasksSidebarNav>
    );
  }

  if (nav.sessionPromptEditing) {
    return (
      <NavSidebar>
        <NavItem
          icon={<ArrowLeft size={15} />}
          label="Back"
          onClick={nav.onSessionPromptBack}
        />
        <NavDivider />
        <div className="sidebar-section-title">Session prompt editor</div>
      </NavSidebar>
    );
  }

  // When in settings view, swap the sidebar contents for the settings menu.
  if (nav.activeView === 'settings') {
    return (
      <SettingsSidebarNav
        activeTab={nav.activeSettingsTab}
        onSelectTab={nav.onSelectSettingsTab}
        onBack={() => goTo('chat')}
      />
    );
  }

  // When editing an agent, swap the sidebar for the agent editor nav.
  if (nav.activeView === 'agents' && nav.agentEditing) {
    return (
      <AgentEditorSidebarNav
        activeTab={nav.agentEditorTab}
        surface={nav.agentEditorSurface}
        onSelectTab={nav.onAgentEditorTabChange}
        onSurfaceChange={nav.onAgentEditorSurfaceChange}
        onBack={nav.onAgentEditorBack}
      />
    );
  }

  // When viewing an agent workspace, show agent session list in the sidebar.
  if (nav.activeView === 'agents' && agentStudio) {
    return (
      <AgentStudioSidebarNav
        agentName={agentStudio.agentName}
        sessions={agentStudio.sessions}
        activeSessionId={agentStudio.activeSessionId}
        loading={agentStudio.loading}
        streamingSessionIds={agentStudio.streamingSessionIds}
        onBack={agentStudio.onBack}
        onEdit={agentStudio.onEdit}
        onNewSession={agentStudio.onNewSession}
        onSelectSession={agentStudio.onSelectSession}
        onDeleteSession={agentStudio.onDeleteSession}
      />
    );
  }

  return (
    <NavSidebar
      footer={
        <NavItem
          icon={<SettingsIcon size={15} />}
          label="Settings"
          active={false}
          onClick={() => goTo('settings')}
        />
      }
    >
      <div className={`sidebar-top-row${searchOpen ? ' sidebar-top-row--search-open' : ''}`}>
        <NavItem
          icon={<Plus size={15} />}
          label="New Chat"
          primary
          onClick={() => {
            goTo('chat');
            chat.onNewChat();
          }}
        />
        <button
          type="button"
          className="sidebar-search-toggle"
          ref={toggleRef}
          onClick={() => {
            if (searchOpen) {
              closeSearch();
            } else {
              setSearchOpen(true);
            }
          }}
          title="Search conversations"
        >
          <Search size={14} />
        </button>
        <div className="sidebar-search-overlay">
          <div className="sidebar-search-overlay-field">
            <input
              type="text"
              className="sidebar-search-overlay-input"
              ref={searchInputRef}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search conversations..."
              onBlur={(e) => {
                if (e.relatedTarget !== toggleRef.current) {
                  closeSearch();
                }
              }}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  closeSearch();
                }
              }}
            />
          </div>
        </div>
      </div>

      {/* 3-6. Feature nav */}
      <NavItem
        icon={<Bot size={15} />}
        label="Agents"
        active={nav.activeView === 'agents'}
        onClick={() => goTo('agents')}
      />
      <NavItem
        icon={<Puzzle size={15} />}
        label="Skills"
        active={nav.activeView === 'skills'}
        onClick={() => goTo('skills')}
      />
      <NavItem
        icon={<BookOpen size={15} />}
        label="Knowledge"
        active={nav.activeView === 'knowledge'}
        onClick={() => goTo('knowledge')}
      />
      <NavItem
        icon={<Zap size={15} />}
        label="Automation"
        active={nav.activeView === 'automation'}
        onClick={() => goTo('automation')}
      />

      <NavDivider />

      {/* Feature sidebar panels or chat session list */}
      <div className="sidebar-chat-region">
        {nav.activeView === 'skills' ? (
          <SkillsSidebarContent />
        ) : nav.activeView === 'knowledge' ? (
          <KnowledgeSidebarContent />
        ) : nav.activeView === 'automation' ? (
          <AutomationSidebarContent />
        ) : (
          <ChatSidebarPanel
            sessions={chat.sessions}
            activeSessionId={chat.activeSessionId}
            streamingSessionIds={chat.streamingSessionIds}
            workspaces={chat.workspaces}
            sessionWorkspaceMap={chat.sessionWorkspaceMap}
            onSelectSession={(id) => {
              goTo('chat');
              chat.onSelectSession(id);
            }}
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
            searchQuery={searchQuery}
            onSearchQueryChange={setSearchQuery}
          />
        )}
      </div>
    </NavSidebar>
  );
}
