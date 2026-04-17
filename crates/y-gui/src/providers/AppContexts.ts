import { createContext, useContext } from 'react';
import type { useChat } from '../hooks/useChat';
import type { useSessions } from '../hooks/useSessions';
import type { useWorkspaces } from '../hooks/useWorkspaces';
import type { useSkills } from '../hooks/useSkills';
import type { useKnowledge } from '../hooks/useKnowledge';
import type { useAgents } from '../hooks/useAgents';
import type { useAutomation } from '../hooks/useAutomation';
import type { useProviders } from '../hooks/useProviders';
import type { useConfig } from '../hooks/useConfig';
import type { ViewType } from '../components/Sidebar';
import type { SettingsTab } from '../components/settings/SettingsPanel';
import type { EditorTab, EditorSurface } from '../components/agents/types';

export const ChatContext = createContext<ReturnType<typeof useChat> | null>(null);
export const SessionsContext = createContext<ReturnType<typeof useSessions> | null>(null);
export const AgentSessionsContext = createContext<ReturnType<typeof useSessions> | null>(null);
export const WorkspacesContext = createContext<ReturnType<typeof useWorkspaces> | null>(null);
export const SkillsContext = createContext<ReturnType<typeof useSkills> | null>(null);
export const KnowledgeContext = createContext<ReturnType<typeof useKnowledge> | null>(null);
export const AgentsContext = createContext<ReturnType<typeof useAgents> | null>(null);
export const AutomationContext = createContext<ReturnType<typeof useAutomation> | null>(null);
export const ProvidersContext = createContext<ReturnType<typeof useProviders> | null>(null);
export const ConfigContext = createContext<ReturnType<typeof useConfig> | null>(null);

export interface NavigationState {
  activeView: ViewType;
  setActiveView: (view: ViewType) => void;
  activeSettingsTab: SettingsTab;
  setActiveSettingsTab: (tab: SettingsTab) => void;
  inputExpanded: boolean;
  setInputExpanded: (expanded: boolean) => void;
  welcomeWorkspaceId: string | null;
  setWelcomeWorkspaceId: (id: string | null) => void;
  diagOpen: boolean;
  setDiagOpen: (open: boolean) => void;
  diagExpanded: boolean;
  setDiagExpanded: (expanded: boolean) => void;
  obsOpen: boolean;
  setObsOpen: (open: boolean) => void;
  obsExpanded: boolean;
  setObsExpanded: (expanded: boolean) => void;

  activeSkillName: string | null;
  setActiveSkillName: (name: string | null) => void;
  importDialogOpen: boolean;
  setImportDialogOpen: (open: boolean) => void;

  selectedKbCollection: string | null;
  setSelectedKbCollection: (name: string | null) => void;

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

  automationSelectedType: 'workflow' | 'schedule' | null;
  setAutomationSelectedType: (type: 'workflow' | 'schedule' | null) => void;
  automationSelectedId: string | null;
  setAutomationSelectedId: (id: string | null) => void;
  automationCreating: 'workflow' | 'schedule' | null;
  setAutomationCreating: (type: 'workflow' | 'schedule' | null) => void;

  onRunWizard?: () => void;
}
export const NavigationContext = createContext<NavigationState | null>(null);

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
export function useNavigationContext() {
  const ctx = useContext(NavigationContext);
  if (!ctx) throw new Error('useNavigationContext must be used within NavigationProvider');
  return ctx;
}
