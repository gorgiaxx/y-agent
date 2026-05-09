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
import type { ViewType } from '../components/Sidebar';
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

export function useChatContext() {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error('useChatContext must be used within ChatProvider');
  return ctx;
}
export function useSessionsContext() {
  const ctx = useContext(SessionsContext);
  if (!ctx) throw new Error('useSessionsContext must be used within SessionsProvider');
  return ctx;
}
export function useAgentSessionsContext() {
  const ctx = useContext(AgentSessionsContext);
  if (!ctx) throw new Error('useAgentSessionsContext must be used within AgentSessionsProvider');
  return ctx;
}
export function useWorkspacesContext() {
  const ctx = useContext(WorkspacesContext);
  if (!ctx) throw new Error('useWorkspacesContext must be used within WorkspacesProvider');
  return ctx;
}
export function useSkillsContext() {
  const ctx = useContext(SkillsContext);
  if (!ctx) throw new Error('useSkillsContext must be used within SkillsProvider');
  return ctx;
}
export function useKnowledgeContext() {
  const ctx = useContext(KnowledgeContext);
  if (!ctx) throw new Error('useKnowledgeContext must be used within KnowledgeProvider');
  return ctx;
}
export function useAgentsContext() {
  const ctx = useContext(AgentsContext);
  if (!ctx) throw new Error('useAgentsContext must be used within AgentsProvider');
  return ctx;
}
export function useAutomationContext() {
  const ctx = useContext(AutomationContext);
  if (!ctx) throw new Error('useAutomationContext must be used within AutomationProvider');
  return ctx;
}
export function useBackgroundTasksContext() {
  const ctx = useContext(BackgroundTasksContext);
  if (!ctx) throw new Error('useBackgroundTasksContext must be used within BackgroundTasksProvider');
  return ctx;
}
export function useProvidersContext() {
  const ctx = useContext(ProvidersContext);
  if (!ctx) throw new Error('useProvidersContext must be used within ProvidersProvider');
  return ctx;
}
export function useConfigContext() {
  const ctx = useContext(ConfigContext);
  if (!ctx) throw new Error('useConfigContext must be used within ConfigProvider');
  return ctx;
}
export function useViewRouting() {
  const ctx = useContext(ViewRoutingContext);
  if (!ctx) throw new Error('useViewRouting must be used within ViewRoutingProvider');
  return ctx;
}
export function usePanelContext() {
  const ctx = useContext(PanelContext);
  if (!ctx) throw new Error('usePanelContext must be used within PanelProvider');
  return ctx;
}
export function useAgentEditorContext() {
  const ctx = useContext(AgentEditorContext);
  if (!ctx) throw new Error('useAgentEditorContext must be used within AgentEditorProvider');
  return ctx;
}
export function useSkillsNavContext() {
  const ctx = useContext(SkillsNavContext);
  if (!ctx) throw new Error('useSkillsNavContext must be used within SkillsNavProvider');
  return ctx;
}
export function useAutomationNavContext() {
  const ctx = useContext(AutomationNavContext);
  if (!ctx) throw new Error('useAutomationNavContext must be used within AutomationNavProvider');
  return ctx;
}
export function useBackgroundTasksNavContext() {
  const ctx = useContext(BackgroundTasksNavContext);
  if (!ctx) throw new Error('useBackgroundTasksNavContext must be used within BackgroundTasksNavProvider');
  return ctx;
}
