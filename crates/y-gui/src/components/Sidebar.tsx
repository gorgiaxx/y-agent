import { useState } from 'react';
import {
  Zap,
  Puzzle,
  BookOpen,
  Bot,
  Settings as SettingsIcon,
  Plus,
} from 'lucide-react';
import type { SessionInfo, WorkspaceInfo } from '../types';
import { ChatSidebarPanel } from './chat-panel/ChatSidebarPanel';
import { NavSidebar, NavItem, NavSearch, NavDivider } from './common/NavSidebar';
import { SettingsSidebarNav } from './settings/SettingsSidebarNav';
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
}

interface SidebarProps {
  chat: ChatSidebarProps;
  nav: NavSidebarPropsGroup;
}

export function Sidebar({ chat, nav }: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState('');

  const goTo = (view: ViewType) => nav.onSelectView(view);

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

  return (
    <NavSidebar
      footer={
        <NavItem
          icon={<SettingsIcon size={15} />}
          label="Settings"
          active={nav.activeView === 'settings'}
          onClick={() => goTo('settings')}
        />
      }
    >
      {/* 1. New Chat */}
      <NavItem
        icon={<Plus size={15} />}
        label="New Chat"
        primary
        onClick={() => {
          goTo('chat');
          chat.onNewChat();
        }}
      />

      {/* 2. Search conversations */}
      <NavSearch
        value={searchQuery}
        onChange={setSearchQuery}
        placeholder="Search conversations..."
      />

      {/* 3-6. Feature nav */}
      <NavItem
        icon={<Puzzle size={15} />}
        label="Skills"
        active={nav.activeView === 'skills'}
        onClick={() => goTo('skills')}
      />
      <NavItem
        icon={<Bot size={15} />}
        label="Agents"
        active={nav.activeView === 'agents'}
        onClick={() => goTo('agents')}
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

      {/* 7+8. Workspaces and chat sessions (master list). */}
      <div className="sidebar-chat-region">
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
          hideHeader
          searchQuery={searchQuery}
          onSearchQueryChange={setSearchQuery}
        />
      </div>
    </NavSidebar>
  );
}
