import { useState, useEffect } from 'react';

import { Sidebar } from '../components/Sidebar';
import { DiagnosticsPanel } from '../components/observation/DiagnosticsPanel';
import { ObservabilityPanel } from '../components/observation/ObservabilityPanel';
import { Activity, Eye } from 'lucide-react';

import { WindowControls } from '../components/ui/WindowControls';

import { ChatView } from '../views/ChatView';
import { SkillsView } from '../views/SkillsView';
import { KnowledgeView } from '../views/KnowledgeView';
import { AgentsView } from '../views/AgentsView';
import { AutomationView } from '../views/AutomationView';
import { SettingsView } from '../views/SettingsView';

import { useViewRouting, usePanelContext, useAgentEditorContext, useSessionsContext, useSkillsContext, useAgentsContext, useWorkspacesContext, useChatContext, useAgentSessionsContext } from '../providers/AppContexts';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useObservability, type TimeRange } from '../hooks/useObservability';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';

export function MainLayout() {
  const viewRouting = useViewRouting();
  const panelCtx = usePanelContext();
  const agentEditor = useAgentEditorContext();
  const sessionHooks = useSessionsContext();
  const skillHooks = useSkillsContext();
  const agentHooks = useAgentsContext();
  const agentSessionHooks = useAgentSessionsContext();
  const workspaceHooks = useWorkspacesContext();
  const chatHooks = useChatContext();

  const diagnosticsScope = resolveDiagnosticsScope(viewRouting.activeView, sessionHooks.activeSessionId);
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
    active: panelCtx.obsOpen,
    timeRange: obsTimeRange,
  });

  // Re-fetch skills whenever the skills view becomes active so newly
  // imported skills appear immediately (the import runs asynchronously
  // and the user may switch views during the long agent execution).
  useEffect(() => {
    if (viewRouting.activeView === 'skills') {
      skillHooks.refresh();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewRouting.activeView]);

  const selectedAgentName = agentEditor.activeAgentId
    ? agentHooks.agents.find((agent) => agent.id === agentEditor.activeAgentId)?.name ?? agentEditor.activeAgentId
    : null;
  const headerTitle = viewRouting.activeView === 'skills'
    ? 'Skills'
    : viewRouting.activeView === 'knowledge'
      ? 'Knowledge Base'
      : viewRouting.activeView === 'agents'
        ? selectedAgentName ?? 'Agents'
        : viewRouting.activeView === 'automation'
          ? 'Automation'
          : sessionHooks.activeSessionId
            ? sessionHooks.sessions.find((s) => s.id === sessionHooks.activeSessionId)?.title || 'Untitled'
            : 'y-agent';

  return (
    <div className="app">
      <Sidebar
        nav={{
          activeView: viewRouting.activeView,
          onSelectView: (v) => {
            viewRouting.setActiveView(v);
            if (v !== 'chat') {
              viewRouting.setInputExpanded(false);
            }
          },
          activeSettingsTab: viewRouting.activeSettingsTab,
          onSelectSettingsTab: (t: string) => viewRouting.setActiveSettingsTab(t as never),
          agentEditing: agentEditor.agentEditing,
          agentEditorTab: agentEditor.agentEditorTab,
          agentEditorSurface: agentEditor.agentEditorSurface,
          onAgentEditorTabChange: agentEditor.setAgentEditorTab,
          onAgentEditorSurfaceChange: agentEditor.onAgentEditorSurfaceChange,
          onAgentEditorBack: agentEditor.onAgentEditorBack,
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
              const fork = await sessionHooks.forkSession(sessionId, Number.MAX_SAFE_INTEGER);
              if (fork) {
                const workspaceId = workspaceHooks.sessionWorkspaceMap[sessionId];
                if (workspaceId) {
                  await workspaceHooks.assignSession(workspaceId, fork.id);
                }
              }
           },
           onRenameSession: async (id, title) => {
              await sessionHooks.renameSession(id, title);
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
        agentStudio={agentEditor.activeAgentId && !agentEditor.agentEditing ? {
          agentName: selectedAgentName ?? agentEditor.activeAgentId,
          sessions: agentSessionHooks.sessions,
          activeSessionId: agentSessionHooks.activeSessionId,
          loading: agentSessionHooks.loading,
          streamingSessionIds: chatHooks.streamingSessionIds,
          onBack: () => agentEditor.setActiveAgentId(null),
          onEdit: () => agentEditor.onAgentStudioEdit(),
          onNewSession: async () => {
            const newSession = await agentSessionHooks.createSession(undefined, { agentId: agentEditor.activeAgentId });
            if (newSession) agentSessionHooks.selectSession(newSession.id);
          },
          onSelectSession: agentSessionHooks.selectSession,
          onDeleteSession: async (id) => {
            await agentSessionHooks.deleteSession(id);
          },
        } : null}
      />

      <main className="main-panel">
        {viewRouting.activeView !== 'settings' && !(viewRouting.activeView === 'agents' && agentEditor.agentEditing) && (
        <header className="main-header" data-tauri-drag-region>
          <div className="main-header-start" data-tauri-drag-region>
            <h1 className="app-title">{headerTitle}</h1>
          </div>
          <div className="header-actions">
            <button
              className={`btn-header ${panelCtx.diagOpen ? 'active' : ''}`}
              onClick={() => panelCtx.setDiagOpen(!panelCtx.diagOpen)}
              title="Diagnostics"
              id="btn-diagnostics"
            >
              <Activity size={16} />
            </button>
            <button
              className={`btn-header ${panelCtx.obsOpen ? 'active' : ''}`}
              onClick={() => panelCtx.setObsOpen(!panelCtx.obsOpen)}
              title="Observability"
              id="btn-observability"
            >
              <Eye size={16} />
            </button>
            <WindowControls />
          </div>
        </header>
        )}

        {viewRouting.activeView === 'chat' && <ChatView />}
        {viewRouting.activeView === 'skills' && <SkillsView />}
        {viewRouting.activeView === 'knowledge' && <KnowledgeView />}
        {viewRouting.activeView === 'agents' && <AgentsView />}
        {viewRouting.activeView === 'automation' && <AutomationView />}
        {viewRouting.activeView === 'settings' && <SettingsView />}
      </main>

      {panelCtx.diagOpen && (
        <DiagnosticsPanel
          entries={diagEntries}
          summary={diagSummary}
          isActive={diagActive}
          isGlobal={diagnosticsScope.isGlobal}
          expanded={panelCtx.diagExpanded}
          sessionId={diagnosticsScope.sessionId}
          onToggleExpand={() => panelCtx.setDiagExpanded(!panelCtx.diagExpanded)}
          onClear={clearDiagnostics}
          onClose={() => panelCtx.setDiagOpen(false)}
        />
      )}

      {panelCtx.obsOpen && (
        <ObservabilityPanel
          snapshot={obsSnapshot}
          loading={obsLoading}
          error={obsError}
          expanded={panelCtx.obsExpanded}
          onToggleExpand={() => panelCtx.setObsExpanded(!panelCtx.obsExpanded)}
          onClose={() => panelCtx.setObsOpen(false)}
          timeRange={obsTimeRange}
          onTimeRangeChange={(r: string) => setObsTimeRange(r as TimeRange)}
        />
      )}
    </div>
  );
}
