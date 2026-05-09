import { useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from 'react';

import { useChat } from '../hooks/useChat';
import { useSessions } from '../hooks/useSessions';
import { useConfig } from '../hooks/useConfig';
import { useWorkspaces } from '../hooks/useWorkspaces';
import { useSkills } from '../hooks/useSkills';
import { useKnowledge } from '../hooks/useKnowledge';
import { useAgents } from '../hooks/useAgents';
import { useAutomation } from '../hooks/useAutomation';
import { useBackgroundTasks } from '../hooks/useBackgroundTasks';
import { useProviders } from '../hooks/useProviders';
import { useThemeProvider, ThemeContext } from '../hooks/useTheme';

import type { ViewType } from '../components/Sidebar';
import type { SettingsTab } from '../components/settings/SettingsPanel';
import type { EditorTab, EditorSurface } from '../components/agents/types';

import {
  ChatContext,
  SessionsContext,
  AgentSessionsContext,
  WorkspacesContext,
  SkillsContext,
  KnowledgeContext,
  AgentsContext,
  AutomationContext,
  BackgroundTasksContext,
  ProvidersContext,
  ConfigContext,
  ViewRoutingContext,
  PanelContext,
  AgentEditorContext,
  SkillsNavContext,
  AutomationNavContext,
  BackgroundTasksNavContext,
} from './AppContexts';
import type {
  ViewRoutingState,
  PanelState,
  AgentEditorState,
  SkillsNavState,
  AutomationNavState,
  BackgroundTasksNavState,
} from './AppContexts';

interface GlobalProvidersProps {
  children: ReactNode;
  onRunWizard?: () => void;
}

export function GlobalProviders({ children, onRunWizard }: GlobalProvidersProps) {
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null);

  const sessionHooks = useSessions();
  const agentSessionHooks = useSessions(activeAgentId);
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
  const [sessionPromptEditing, setSessionPromptEditing] = useState(false);
  const [sessionPromptSessionId, setSessionPromptSessionId] = useState<string | null>(null);
  const [backgroundTasksSidebarOpen, setBackgroundTasksSidebarOpen] = useState(false);
  const [backgroundTasksSessionId, setBackgroundTasksSessionId] = useState<string | null>(null);
  const [welcomeWorkspaceId, setWelcomeWorkspaceId] = useState<string | null>(null);
  const [diagOpen, setDiagOpen] = useState(false);
  const [diagExpanded, setDiagExpanded] = useState(false);
  const [obsOpen, setObsOpen] = useState(false);
  const [obsExpanded, setObsExpanded] = useState(false);

  const [activeSkillName, setActiveSkillName] = useState<string | null>(null);
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [agentEditing, setAgentEditing] = useState(false);
  const [agentEditorTab, setAgentEditorTab] = useState<EditorTab>('general');
  const [agentEditorSurface, setAgentEditorSurface] = useState<EditorSurface>('form');
  const agentEditorSurfaceHandlerRef = useRef<((surface: EditorSurface) => void) | null>(null);
  const agentStudioEditHandlerRef = useRef<(() => void) | null>(null);
  const onAgentEditorSurfaceChange = useCallback((surface: EditorSurface) => {
    if (agentEditorSurfaceHandlerRef.current) {
      agentEditorSurfaceHandlerRef.current(surface);
    } else {
      setAgentEditorSurface(surface);
    }
  }, []);
  const setAgentEditorSurfaceHandler = useCallback((handler: ((surface: EditorSurface) => void) | null) => {
    agentEditorSurfaceHandlerRef.current = handler;
  }, []);
  const onAgentStudioEdit = useCallback(() => {
    agentStudioEditHandlerRef.current?.();
  }, []);
  const setAgentStudioEditHandler = useCallback((handler: (() => void) | null) => {
    agentStudioEditHandlerRef.current = handler;
  }, []);
  const [automationSelectedType, setAutomationSelectedType] = useState<'workflow' | 'schedule' | null>(null);
  const [automationSelectedId, setAutomationSelectedId] = useState<string | null>(null);
  const [automationCreating, setAutomationCreating] = useState<'workflow' | 'schedule' | null>(null);
  const [selectedBackgroundTaskId, setSelectedBackgroundTaskId] = useState<string | null>(null);

  const automationHooks = useAutomation(activeView === 'automation');
  const currentBackgroundTasksSessionId = activeView === 'agents'
    ? agentSessionHooks.activeSessionId
    : sessionHooks.activeSessionId;
  const observedBackgroundTasksSessionId = backgroundTasksSidebarOpen
    ? backgroundTasksSessionId ?? currentBackgroundTasksSessionId
    : currentBackgroundTasksSessionId;
  const backgroundTaskHooks = useBackgroundTasks(observedBackgroundTasksSessionId);
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

  const viewRouting = useMemo<ViewRoutingState>(() => ({
    activeView, setActiveView,
    activeSettingsTab, setActiveSettingsTab,
    inputExpanded, setInputExpanded,
    sessionPromptEditing, setSessionPromptEditing,
    sessionPromptSessionId, setSessionPromptSessionId,
    backgroundTasksSidebarOpen, setBackgroundTasksSidebarOpen,
    backgroundTasksSessionId, setBackgroundTasksSessionId,
    welcomeWorkspaceId, setWelcomeWorkspaceId,
    onRunWizard,
  }), [
    activeView,
    activeSettingsTab,
    inputExpanded,
    sessionPromptEditing,
    sessionPromptSessionId,
    backgroundTasksSidebarOpen,
    backgroundTasksSessionId,
    welcomeWorkspaceId,
    onRunWizard,
  ]);

  const panelState = useMemo<PanelState>(() => ({
    diagOpen, setDiagOpen,
    diagExpanded, setDiagExpanded,
    obsOpen, setObsOpen,
    obsExpanded, setObsExpanded,
  }), [diagOpen, diagExpanded, obsOpen, obsExpanded]);

  const agentEditorState = useMemo<AgentEditorState>(() => ({
    activeAgentId, setActiveAgentId,
    agentEditing, agentEditorTab, agentEditorSurface,
    setAgentEditing, setAgentEditorTab, setAgentEditorSurface,
    onAgentEditorSurfaceChange, setAgentEditorSurfaceHandler,
    onAgentEditorBack: () => { setAgentEditing(false); },
    onAgentStudioEdit, setAgentStudioEditHandler,
  }), [activeAgentId, agentEditing, agentEditorTab, agentEditorSurface,
       onAgentEditorSurfaceChange, setAgentEditorSurfaceHandler,
       onAgentStudioEdit, setAgentStudioEditHandler]);

  const skillsNavState = useMemo<SkillsNavState>(() => ({
    activeSkillName, setActiveSkillName,
    importDialogOpen, setImportDialogOpen,
  }), [activeSkillName, importDialogOpen]);

  const automationNavState = useMemo<AutomationNavState>(() => ({
    automationSelectedType, setAutomationSelectedType,
    automationSelectedId, setAutomationSelectedId,
    automationCreating, setAutomationCreating,
  }), [automationSelectedType, automationSelectedId, automationCreating]);

  const backgroundTasksNavState = useMemo<BackgroundTasksNavState>(() => ({
    selectedBackgroundTaskId,
    setSelectedBackgroundTaskId,
  }), [selectedBackgroundTaskId]);

  return (
    <ThemeContext.Provider value={themeCtx}>
      <ConfigContext.Provider value={configHooks}>
        <SessionsContext.Provider value={sessionHooks}>
          <AgentSessionsContext.Provider value={agentSessionHooks}>
            <ChatContext.Provider value={chatHooks}>
              <WorkspacesContext.Provider value={workspaceHooks}>
                <SkillsContext.Provider value={skillHooks}>
                  <KnowledgeContext.Provider value={knowledgeHooks}>
                    <AgentsContext.Provider value={agentHooks}>
                      <AutomationContext.Provider value={automationHooks}>
                        <BackgroundTasksContext.Provider value={backgroundTaskHooks}>
                          <ProvidersContext.Provider value={providerHooks}>
                            <ViewRoutingContext.Provider value={viewRouting}>
                              <PanelContext.Provider value={panelState}>
                                <AgentEditorContext.Provider value={agentEditorState}>
                                  <SkillsNavContext.Provider value={skillsNavState}>
                                    <AutomationNavContext.Provider value={automationNavState}>
                                      <BackgroundTasksNavContext.Provider value={backgroundTasksNavState}>
                                        {children}
                                      </BackgroundTasksNavContext.Provider>
                                    </AutomationNavContext.Provider>
                                  </SkillsNavContext.Provider>
                                </AgentEditorContext.Provider>
                              </PanelContext.Provider>
                            </ViewRoutingContext.Provider>
                          </ProvidersContext.Provider>
                        </BackgroundTasksContext.Provider>
                      </AutomationContext.Provider>
                    </AgentsContext.Provider>
                  </KnowledgeContext.Provider>
                </SkillsContext.Provider>
              </WorkspacesContext.Provider>
            </ChatContext.Provider>
          </AgentSessionsContext.Provider>
        </SessionsContext.Provider>
      </ConfigContext.Provider>
    </ThemeContext.Provider>
  );
}
