import { useState, useEffect } from 'react';

import { Sidebar } from '../components/Sidebar';
import { DiagnosticsPanel } from '../components/observation/DiagnosticsPanel';
import { ObservabilityPanel } from '../components/observation/ObservabilityPanel';
import { Activity, ArrowLeft, Eye } from 'lucide-react';

import { ChatView } from '../views/ChatView';
import { SkillsView } from '../views/SkillsView';
import { KnowledgeView } from '../views/KnowledgeView';
import { AgentsView } from '../views/AgentsView';
import { AutomationView } from '../views/AutomationView';
import { SettingsView } from '../views/SettingsView';
import { type SettingsTab } from '../components/settings/SettingsPanel';

import { useNavigationContext, useSessionsContext, useSkillsContext, useAgentsContext, useWorkspacesContext, useKnowledgeContext, useAutomationContext, useChatContext } from '../providers/AppContexts';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useObservability, type TimeRange } from '../hooks/useObservability';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';

export function MainLayout() {
  const navProps = useNavigationContext();
  const sessionHooks = useSessionsContext();
  const skillHooks = useSkillsContext();
  const agentHooks = useAgentsContext();
  const workspaceHooks = useWorkspacesContext();
  const knowledgeHooks = useKnowledgeContext();
  const autoHooks = useAutomationContext();
  const chatHooks = useChatContext();

  const diagnosticsScope = resolveDiagnosticsScope(navProps.activeView, sessionHooks.activeSessionId);
  const {
    entries: diagEntries,
    summary: diagSummary,
    isActive: diagActive,
    clear: clearDiagnostics,
  } = useDiagnostics(diagnosticsScope.sessionId);

  const [obsTimeRange, setObsTimeRange] = useState<TimeRange>('all');
  const {
    snapshot: obsSnapshot,
    loading: obsLoading,
    error: obsError,
  } = useObservability({
    active: navProps.obsOpen,
    timeRange: obsTimeRange,
  });

  // Re-fetch skills whenever the skills view becomes active so newly
  // imported skills appear immediately (the import runs asynchronously
  // and the user may switch views during the long agent execution).
  useEffect(() => {
    if (navProps.activeView === 'skills') {
      skillHooks.refresh();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [navProps.activeView]);

  const selectedAgentName = navProps.activeAgentId
    ? agentHooks.agents.find((agent) => agent.id === navProps.activeAgentId)?.name ?? navProps.activeAgentId
    : null;
  const showAgentBack = navProps.activeView === 'agents' && !!navProps.activeAgentId;
  const headerTitle = navProps.activeView === 'skills'
    ? 'Skills'
    : navProps.activeView === 'knowledge'
      ? 'Knowledge Base'
      : navProps.activeView === 'agents'
        ? selectedAgentName ?? 'Agents'
        : navProps.activeView === 'automation'
          ? 'Automation'
          : sessionHooks.activeSessionId
            ? sessionHooks.sessions.find((s) => s.id === sessionHooks.activeSessionId)?.title || 'Untitled'
            : 'y-agent';

  return (
    <div className="app">
      <Sidebar
        nav={{
          activeView: navProps.activeView,
          onSelectView: (v) => {
            navProps.setActiveView(v);
            if (v !== 'chat') {
              navProps.setInputExpanded(false);
            }
          },
          activeSettingsTab: navProps.activeSettingsTab,
          onSelectSettingsTab: (t: string) => navProps.setActiveSettingsTab(t as SettingsTab),
        }}
        chat={{
          sessions: sessionHooks.sessions,
          activeSessionId: sessionHooks.activeSessionId,
          onSelectSession: sessionHooks.selectSession,
          workspaces: workspaceHooks.workspaces,
          onAssignSession: workspaceHooks.assignSession,
          streamingSessionIds: chatHooks.streamingSessionIds,
          sessionWorkspaceMap: workspaceHooks.sessionWorkspaceMap,
          onNewChat: async () => {
             const newSession = await sessionHooks.createSession();
             if (newSession) sessionHooks.selectSession(newSession.id);
          },
          onNewChatInWorkspace: async (workspaceId: string) => {
             const newSession = await sessionHooks.createSession();
             if (newSession) {
               await workspaceHooks.assignSession(workspaceId, newSession.id);
               sessionHooks.selectSession(newSession.id);
             }
          },
          onDeleteSession: async (id) => {
             await sessionHooks.deleteSession(id);
          },
          onForkSession: async (sessionId) => {
             // Fork the entire session (use max index to copy all messages).
             const fork = await sessionHooks.forkSession(sessionId, Number.MAX_SAFE_INTEGER);
             // If the original session belongs to a workspace, assign the fork to the same workspace.
             if (fork) {
               const workspaceId = workspaceHooks.sessionWorkspaceMap[sessionId];
               if (workspaceId) {
                 await workspaceHooks.assignSession(workspaceId, fork.id);
               }
             }
          },
          onCreateWorkspace: async (name, path) => {
             await workspaceHooks.createWorkspace(name, path);
          },
          onUpdateWorkspace: async (id, name, path) => {
             await workspaceHooks.updateWorkspace(id, name, path);
          },
          onDeleteWorkspace: async (id) => {
             await workspaceHooks.deleteWorkspace(id);
          },
          onUnassignSession: async (sessionId) => {
             await workspaceHooks.unassignSession(sessionId);
          },
        }}
        skills={{
          skills: skillHooks.skills,
          activeSkillName: navProps.activeSkillName,
          onSelectSkill: navProps.setActiveSkillName,
          onImportClick: () => navProps.setImportDialogOpen(true),
          importStatus: skillHooks.importStatus,
          importError: skillHooks.importError,
          onClearImportStatus: skillHooks.clearImportStatus,
        }}
        knowledge={{
          collections: knowledgeHooks.collections,
          selectedCollection: navProps.selectedKbCollection,
          onSelectCollection: navProps.setSelectedKbCollection,
          onCreateCollection: knowledgeHooks.createCollection,
          ingestStatus: knowledgeHooks.ingestStatus,
          batchProgress: knowledgeHooks.batchProgress,
          ingestError: knowledgeHooks.ingestError,
          onClearIngestStatus: knowledgeHooks.clearIngestStatus,
          onCancelIngest: knowledgeHooks.cancelIngest,
        }}
        automation={{
          workflows: autoHooks.workflows,
          schedules: autoHooks.schedules,
          selectedType: navProps.automationSelectedType,
          selectedId: navProps.automationSelectedId,
          onSelectWorkflow: (id: string) => {
            navProps.setAutomationSelectedType('workflow');
            navProps.setAutomationSelectedId(id);
            navProps.setAutomationCreating(null);
          },
          onSelectSchedule: (id: string) => {
            navProps.setAutomationSelectedType('schedule');
            navProps.setAutomationSelectedId(id);
            navProps.setAutomationCreating(null);
          },
          onCreateWorkflow: () => {
            navProps.setAutomationSelectedType(null);
            navProps.setAutomationSelectedId(null);
            navProps.setAutomationCreating('workflow');
          },
          onCreateSchedule: () => {
            navProps.setAutomationSelectedType(null);
            navProps.setAutomationSelectedId(null);
            navProps.setAutomationCreating('schedule');
          },
        }}
      />

      <main className="main-panel">
        {navProps.activeView !== 'settings' && (
        <header className="main-header">
          <div className="main-header-start">
            {showAgentBack && (
              <button
                className="btn-header"
                onClick={() => navProps.setActiveAgentId(null)}
                title="Back to agents"
                aria-label="Back to agents"
              >
                <ArrowLeft size={16} />
              </button>
            )}
            <h1 className="app-title">{headerTitle}</h1>
          </div>
          <div className="header-actions">
            <button
              className={`btn-header ${navProps.diagOpen ? 'active' : ''}`}
              onClick={() => navProps.setDiagOpen(!navProps.diagOpen)}
              title="Diagnostics"
              id="btn-diagnostics"
            >
              <Activity size={16} />
            </button>
            <button
              className={`btn-header ${navProps.obsOpen ? 'active' : ''}`}
              onClick={() => navProps.setObsOpen(!navProps.obsOpen)}
              title="Observability"
              id="btn-observability"
            >
              <Eye size={16} />
            </button>
          </div>
        </header>
        )}

        {navProps.activeView === 'chat' && <ChatView />}
        {navProps.activeView === 'skills' && <SkillsView />}
        {navProps.activeView === 'knowledge' && <KnowledgeView />}
        {navProps.activeView === 'agents' && <AgentsView />}
        {navProps.activeView === 'automation' && <AutomationView />}
        {navProps.activeView === 'settings' && <SettingsView />}
      </main>

      {navProps.diagOpen && (
        <DiagnosticsPanel
          entries={diagEntries}
          summary={diagSummary}
          isActive={diagActive}
          isGlobal={diagnosticsScope.isGlobal}
          expanded={navProps.diagExpanded}
          sessionId={diagnosticsScope.sessionId}
          onToggleExpand={() => navProps.setDiagExpanded(!navProps.diagExpanded)}
          onClear={clearDiagnostics}
          onClose={() => navProps.setDiagOpen(false)}
        />
      )}

      {navProps.obsOpen && (
        <ObservabilityPanel
          snapshot={obsSnapshot}
          loading={obsLoading}
          error={obsError}
          expanded={navProps.obsExpanded}
          onToggleExpand={() => navProps.setObsExpanded(!navProps.obsExpanded)}
          onClose={() => navProps.setObsOpen(false)}
          timeRange={obsTimeRange}
          onTimeRangeChange={(r: string) => setObsTimeRange(r as TimeRange)}
        />
      )}
    </div>
  );
}
