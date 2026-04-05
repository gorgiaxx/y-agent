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

interface GlobalProvidersProps {
  children: ReactNode;
  onRunWizard?: () => void;
}

export function GlobalProviders({ children, onRunWizard }: GlobalProvidersProps) {
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

  // Load messages when active session changes.
  // IMPORTANT: depend on specific values, not entire hook objects.
  // Using [sessionHooks, chatHooks] causes infinite re-renders because
  // hook return objects get new references on every render.
  const { activeSessionId } = sessionHooks;
  const { loadMessages, clearMessages } = chatHooks;
  useEffect(() => {
    if (activeSessionId) {
      loadMessages(activeSessionId);
    } else {
      clearMessages();
    }
  }, [activeSessionId, loadMessages, clearMessages]);

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
    selectedKbCollection: knowledgeHooks.selectedCollection,
    setSelectedKbCollection: knowledgeHooks.setSelectedCollection,
    activeAgentId, setActiveAgentId,
    automationSelectedType, setAutomationSelectedType,
    automationSelectedId, setAutomationSelectedId,
    automationCreating, setAutomationCreating,
    onRunWizard,
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
