import { useState, useEffect, type ReactNode } from 'react';

import { useChat } from '../hooks/useChat';
import { useSessions } from '../hooks/useSessions';
import { useConfig } from '../hooks/useConfig';
import { useWorkspaces } from '../hooks/useWorkspaces';
import { useSkills } from '../hooks/useSkills';
import { useKnowledge } from '../hooks/useKnowledge';
import { useAgents } from '../hooks/useAgents';
import { useAutomation } from '../hooks/useAutomation';
import { useProviders } from '../hooks/useProviders';
import { useThemeProvider, ThemeContext } from '../hooks/useTheme';

import type { ViewType } from '../components/Sidebar';
import type { SettingsTab } from '../components/settings/SettingsPanel';

import {
  ChatContext,
  SessionsContext,
  WorkspacesContext,
  SkillsContext,
  KnowledgeContext,
  AgentsContext,
  AutomationContext,
  ProvidersContext,
  ConfigContext,
  NavigationContext,
} from './AppContexts';

export function GlobalProviders({ children }: { children: ReactNode }) {
  const sessionHooks = useSessions();
  const chatHooks = useChat(sessionHooks.activeSessionId);
  const configHooks = useConfig();
  const themeCtx = useThemeProvider(configHooks.config.theme);
  const workspaceHooks = useWorkspaces();
  const skillHooks = useSkills();
  const knowledgeHooks = useKnowledge();
  const agentHooks = useAgents();

  const [activeView, setActiveView] = useState<ViewType>('chat');
  const [activeSettingsTab, setActiveSettingsTab] = useState<SettingsTab>('general');
  const [inputExpanded, setInputExpanded] = useState(false);
  const [welcomeWorkspaceId, setWelcomeWorkspaceId] = useState<string | null>(null);
  const [diagOpen, setDiagOpen] = useState(false);
  const [diagExpanded, setDiagExpanded] = useState(false);
  const [obsOpen, setObsOpen] = useState(false);
  const [obsExpanded, setObsExpanded] = useState(false);

  const [activeSkillName, setActiveSkillName] = useState<string | null>(null);
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [selectedKbCollection, setSelectedKbCollection] = useState<string | null>(null);
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null);
  const [automationSelectedType, setAutomationSelectedType] = useState<'workflow' | 'schedule' | null>(null);
  const [automationSelectedId, setAutomationSelectedId] = useState<string | null>(null);
  const [automationCreating, setAutomationCreating] = useState<'workflow' | 'schedule' | null>(null);

  const automationHooks = useAutomation(activeView === 'automation');
  const providerHooks = useProviders(configHooks.loadSection);

  // Default welcome workspace to first workspace (alphabetically).
  useEffect(() => {
    if (workspaceHooks.workspaces.length > 0 && !welcomeWorkspaceId) {
      const sorted = [...workspaceHooks.workspaces].sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
      );
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setWelcomeWorkspaceId(sorted[0].id);
    }
  }, [workspaceHooks.workspaces, welcomeWorkspaceId]);

  // Load messages when active session changes
  useEffect(() => {
    if (sessionHooks.activeSessionId) {
      chatHooks.loadMessages(sessionHooks.activeSessionId);
    } else {
      chatHooks.clearMessages();
    }
  }, [sessionHooks, chatHooks]);

  const navState = {
    activeView, setActiveView,
    activeSettingsTab, setActiveSettingsTab,
    inputExpanded, setInputExpanded,
    welcomeWorkspaceId, setWelcomeWorkspaceId,
    diagOpen, setDiagOpen,
    diagExpanded, setDiagExpanded,
    obsOpen, setObsOpen,
    obsExpanded, setObsExpanded,
    activeSkillName, setActiveSkillName,
    importDialogOpen, setImportDialogOpen,
    selectedKbCollection, setSelectedKbCollection,
    activeAgentId, setActiveAgentId,
    automationSelectedType, setAutomationSelectedType,
    automationSelectedId, setAutomationSelectedId,
    automationCreating, setAutomationCreating,
  };

  return (
    <ThemeContext.Provider value={themeCtx}>
      <ConfigContext.Provider value={configHooks}>
        <SessionsContext.Provider value={sessionHooks}>
          <ChatContext.Provider value={chatHooks}>
            <WorkspacesContext.Provider value={workspaceHooks}>
              <SkillsContext.Provider value={skillHooks}>
                <KnowledgeContext.Provider value={knowledgeHooks}>
                  <AgentsContext.Provider value={agentHooks}>
                    <AutomationContext.Provider value={automationHooks}>
                      <ProvidersContext.Provider value={providerHooks}>
                        <NavigationContext.Provider value={navState}>
                          {children}
                        </NavigationContext.Provider>
                      </ProvidersContext.Provider>
                    </AutomationContext.Provider>
                  </AgentsContext.Provider>
                </KnowledgeContext.Provider>
              </SkillsContext.Provider>
            </WorkspacesContext.Provider>
          </ChatContext.Provider>
        </SessionsContext.Provider>
      </ConfigContext.Provider>
    </ThemeContext.Provider>
  );
}
