import { createContext, useContext } from 'react';
import type { UseChatReturn } from '../hooks/useChat';
import type { UseSessionsReturn } from '../hooks/useSessions';
import type { UseWorkspacesReturn } from '../hooks/useWorkspaces';
import type { UseSkillsReturn } from '../hooks/useSkills';
import type { UseKnowledgeReturn } from '../hooks/useKnowledge';
import type { UseAgentsReturn } from '../hooks/useAgents';
import type { UseAutomationReturn } from '../hooks/useAutomation';
import type { UseBackgroundTasksReturn } from '../hooks/useBackgroundTasks';
import type { UseProvidersReturn } from '../hooks/useProviders';
import type { UseConfigReturn } from '../hooks/useConfig';
import type { ViewType } from '../types';
import type { SettingsTab } from '../components/settings/SettingsPanel';
import type { EditorTab, EditorSurface } from '../components/agents/types';

export const ChatContext = createContext<UseChatReturn | null>(null);
export const SessionsContext = createContext<UseSessionsReturn | null>(null);
export const AgentSessionsContext = createContext<UseSessionsReturn | null>(null);
export const WorkspacesContext = createContext<UseWorkspacesReturn | null>(null);
export const SkillsContext = createContext<UseSkillsReturn | null>(null);
export const KnowledgeContext = createContext<UseKnowledgeReturn | null>(null);
export const AgentsContext = createContext<UseAgentsReturn | null>(null);
export const AutomationContext = createContext<UseAutomationReturn | null>(null);
export const BackgroundTasksContext = createContext<UseBackgroundTasksReturn | null>(null);
export const ProvidersContext = createContext<UseProvidersReturn | null>(null);
export const ConfigContext = createContext<UseConfigReturn | null>(null);

// -- Focused sub-contexts for navigation state --

export interface ViewRoutingState {
  activeView: ViewType;
  setActiveView: (view: ViewType) => void;
  activeSettingsTab: SettingsTab;
  setActiveSettingsTab: (tab: SettingsTab) => void;
  inputExpanded: boolean;
  setInputExpanded: (expanded: boolean) => void;
  sessionPromptEditing: boolean;
  setSessionPromptEditing: (editing: boolean) => void;
  sessionPromptSessionId: string | null;
  setSessionPromptSessionId: (id: string | null) => void;
  backgroundTasksSidebarOpen: boolean;
  setBackgroundTasksSidebarOpen: (open: boolean) => void;
  backgroundTasksSessionId: string | null;
  setBackgroundTasksSessionId: (id: string | null) => void;
  welcomeWorkspaceId: string | null;
  setWelcomeWorkspaceId: (id: string | null) => void;
  onRunWizard?: () => void;
}

export interface PanelState {
  diagOpen: boolean;
  setDiagOpen: (open: boolean) => void;
  diagExpanded: boolean;
  setDiagExpanded: (expanded: boolean) => void;
  obsOpen: boolean;
  setObsOpen: (open: boolean) => void;
  obsExpanded: boolean;
  setObsExpanded: (expanded: boolean) => void;
}

export interface AgentEditorState {
  activeAgentId: string | null;
  setActiveAgentId: (id: string | null) => void;
  agentEditing: boolean;
  agentEditorTab: EditorTab;
  agentEditorSurface: EditorSurface;
  setAgentEditing: (editing: boolean) => void;
  setAgentEditorTab: (tab: EditorTab) => void;
  setAgentEditorSurface: (surface: EditorSurface) => void;
  onAgentEditorSurfaceChange: (surface: EditorSurface) => void;
  setAgentEditorSurfaceHandler: (handler: ((surface: EditorSurface) => void) | null) => void;
  onAgentEditorBack: () => void;
  onAgentStudioEdit: () => void;
  setAgentStudioEditHandler: (handler: (() => void) | null) => void;
}

export interface SkillsNavState {
  activeSkillName: string | null;
  setActiveSkillName: (name: string | null) => void;
  importDialogOpen: boolean;
  setImportDialogOpen: (open: boolean) => void;
}

export interface AutomationNavState {
  automationSelectedType: 'workflow' | 'schedule' | null;
  setAutomationSelectedType: (type: 'workflow' | 'schedule' | null) => void;
  automationSelectedId: string | null;
  setAutomationSelectedId: (id: string | null) => void;
  automationCreating: 'workflow' | 'schedule' | null;
  setAutomationCreating: (type: 'workflow' | 'schedule' | null) => void;
}

export interface BackgroundTasksNavState {
  selectedBackgroundTaskId: string | null;
  setSelectedBackgroundTaskId: (id: string | null) => void;
}

export const ViewRoutingContext = createContext<ViewRoutingState | null>(null);
export const PanelContext = createContext<PanelState | null>(null);
export const AgentEditorContext = createContext<AgentEditorState | null>(null);
export const SkillsNavContext = createContext<SkillsNavState | null>(null);
export const AutomationNavContext = createContext<AutomationNavState | null>(null);
export const BackgroundTasksNavContext = createContext<BackgroundTasksNavState | null>(null);

function createSafeHook<T>(ctx: React.Context<T | null>, name: string): () => T {
  return () => {
    const value = useContext(ctx);
    if (!value) throw new Error(`${name} must be used within its Provider`);
    return value;
  };
}

export const useChatContext = createSafeHook(ChatContext, 'useChatContext');
export const useSessionsContext = createSafeHook(SessionsContext, 'useSessionsContext');
export const useAgentSessionsContext = createSafeHook(AgentSessionsContext, 'useAgentSessionsContext');
export const useWorkspacesContext = createSafeHook(WorkspacesContext, 'useWorkspacesContext');
export const useSkillsContext = createSafeHook(SkillsContext, 'useSkillsContext');
export const useKnowledgeContext = createSafeHook(KnowledgeContext, 'useKnowledgeContext');
export const useAgentsContext = createSafeHook(AgentsContext, 'useAgentsContext');
export const useAutomationContext = createSafeHook(AutomationContext, 'useAutomationContext');
export const useBackgroundTasksContext = createSafeHook(BackgroundTasksContext, 'useBackgroundTasksContext');
export const useProvidersContext = createSafeHook(ProvidersContext, 'useProvidersContext');
export const useConfigContext = createSafeHook(ConfigContext, 'useConfigContext');
export const useViewRouting = createSafeHook(ViewRoutingContext, 'useViewRouting');
export const usePanelContext = createSafeHook(PanelContext, 'usePanelContext');
export const useAgentEditorContext = createSafeHook(AgentEditorContext, 'useAgentEditorContext');
export const useSkillsNavContext = createSafeHook(SkillsNavContext, 'useSkillsNavContext');
export const useAutomationNavContext = createSafeHook(AutomationNavContext, 'useAutomationNavContext');
export const useBackgroundTasksNavContext = createSafeHook(BackgroundTasksNavContext, 'useBackgroundTasksNavContext');
