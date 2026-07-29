import { useState, useEffect } from 'react';

import { Sidebar } from '../components/Sidebar';
import { DiagnosticsPanel } from '../components/observation/DiagnosticsPanel';
import { ObservabilityPanel } from '../components/observation/ObservabilityPanel';
import { InfoPanel } from '../components/observation/InfoPanel';
import { Activity, Eye, Info, ChevronLeft } from 'lucide-react';

import { WindowControls } from '../components/ui/WindowControls';

import { ChatView } from '../views/ChatView';
import { SessionPromptPanel } from '../components/chat-panel/SessionPromptPanel';
import { SkillsView } from '../views/SkillsView';
import { KnowledgeView } from '../views/KnowledgeView';
import { AgentsView } from '../views/AgentsView';
import { AutomationView } from '../views/AutomationView';
import { BackgroundTasksView } from '../views/BackgroundTasksView';
import { SettingsView } from '../views/SettingsView';
import type { SettingsTab } from '../components/settings/SettingsPanel';
import type { ViewType } from '../types';

import {
  useViewRouting,
  usePanelContext,
  useAgentEditorContext,
  useSessionsContext,
  useSkillsContext,
  useAgentsContext,
  useWorkspacesContext,
  useChatContext,
  useAgentSessionsContext,
  useConfigContext,
} from '../providers/AppContexts';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useObservability, type TimeRange } from '../hooks/useObservability';
import { useInfoPanel } from '../hooks/useInfoPanel';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';
import { useKeyboardShortcuts } from '../shortcuts/useKeyboardShortcuts';

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
  const configHooks = useConfigContext();

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
    memoryStats: obsMemoryStats,
    loading: obsLoading,
    error: obsError,
  } = useObservability({
    active: panelCtx.obsOpen,
    timeRange: obsTimeRange,
  });

  const infoData = useInfoPanel();

  // Drill-in breadcrumb stack for sub-agent (plan phase / loop round) sessions.
  // Each entry is a session we drilled INTO, remembering the parent to return to.
  const [drillStack, setDrillStack] = useState<Array<{ parentId: string; childId: string; title: string }>>([]);

  const openChildSession = (childId: string, title: string) => {
    const parentId = sessionHooks.activeSessionId;
    if (!parentId || childId === parentId) return;
    setDrillStack((prev) => [...prev, { parentId, childId, title }]);
    sessionHooks.selectSession(childId);
  };

  const drillBack = () => {
    setDrillStack((prev) => {
      if (prev.length === 0) return prev;
      const target = prev[prev.length - 1];
      sessionHooks.selectSession(target.parentId);
      return prev.slice(0, -1);
    });
  };

  // If the user navigates elsewhere (active session no longer the drilled child),
  // clear the stack so a stale breadcrumb never lingers.
  useEffect(() => {
    if (drillStack.length === 0) return;
    const top = drillStack[drillStack.length - 1];
    if (sessionHooks.activeSessionId !== top.childId) {
      setDrillStack([]);
    }
  }, [sessionHooks.activeSessionId, drillStack]);

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
  const drilledInto = viewRouting.activeView === 'chat' && drillStack.length > 0
    ? drillStack[drillStack.length - 1]
    : null;

  const selectView = (view: ViewType) => {
    viewRouting.setActiveView(view);
    viewRouting.setBackgroundTasksSidebarOpen(false);
    viewRouting.setSessionPromptEditing(false);
    viewRouting.setSessionPromptSessionId(null);
    if (view !== 'chat') viewRouting.setInputExpanded(false);
  };

  const handleNewChat = async () => {
    const activeWorkspacePath = sessionHooks.sessions.find(
      (session) => session.id === sessionHooks.activeSessionId,
    )?.workspace_path;
    const welcomeWorkspacePath = workspaceHooks.workspaces.find(
      (workspace) => workspace.id === viewRouting.welcomeWorkspaceId,
    )?.path;
    const newSession = await sessionHooks.createSession(undefined, {
      workspacePath: activeWorkspacePath ?? welcomeWorkspacePath ?? null,
    });
    if (!newSession) return;

    const workspaceId = sessionHooks.activeSessionId
      ? workspaceHooks.sessionWorkspaceMap[sessionHooks.activeSessionId]
      : viewRouting.welcomeWorkspaceId;
    if (workspaceId) await workspaceHooks.assignSession(workspaceId, newSession.id);
    sessionHooks.selectSession(newSession.id);
  };

  useKeyboardShortcuts(configHooks.config.keyboard_shortcuts, {
    new_chat: async () => {
      selectView('chat');
      await handleNewChat();
    },
    focus_composer: () => {
      selectView('chat');
      requestAnimationFrame(() => {
        document.querySelector<HTMLElement>('[data-shortcut-target="composer"]')?.focus();
      });
    },
    open_settings: () => selectView('settings'),
    keyboard_shortcuts: () => {
      viewRouting.setActiveSettingsTab('keyboardShortcuts');
      selectView('settings');
    },
    show_chat: () => selectView('chat'),
    show_automation: () => selectView('automation'),
    show_skills: () => selectView('skills'),
    show_knowledge: () => selectView('knowledge'),
    show_agents: () => selectView('agents'),
    toggle_diagnostics: () => panelCtx.setDiagOpen(!panelCtx.diagOpen),
    toggle_observability: () => panelCtx.setObsOpen(!panelCtx.obsOpen),
    toggle_info: () => panelCtx.setInfoOpen(!panelCtx.infoOpen),
  });

  const headerTitle = viewRouting.backgroundTasksSidebarOpen
    ? 'Tasks'
    : viewRouting.activeView === 'skills'
      ? 'Skills'
      : viewRouting.activeView === 'knowledge'
        ? 'Knowledge Base'
        : viewRouting.activeView === 'agents'
          ? selectedAgentName ?? 'Agents'
          : viewRouting.activeView === 'automation'
            ? 'Automation'
            : drilledInto
              ? drilledInto.title
              : sessionHooks.activeSessionId
                ? sessionHooks.sessions.find((s) => s.id === sessionHooks.activeSessionId)?.title || 'Untitled'
                : 'y-agent';

  return (
    <div className="app">
      <Sidebar
        nav={{
          activeView: viewRouting.activeView,
          onSelectView: selectView,
          activeSettingsTab: viewRouting.activeSettingsTab,
          onSelectSettingsTab: (t: string) => viewRouting.setActiveSettingsTab(t as SettingsTab),
          agentEditing: agentEditor.agentEditing,
          agentEditorTab: agentEditor.agentEditorTab,
          agentEditorSurface: agentEditor.agentEditorSurface,
          onAgentEditorTabChange: agentEditor.setAgentEditorTab,
          onAgentEditorSurfaceChange: agentEditor.onAgentEditorSurfaceChange,
          onAgentEditorBack: agentEditor.onAgentEditorBack,
          sessionPromptEditing: viewRouting.sessionPromptEditing,
          onSessionPromptBack: () => {
            viewRouting.setSessionPromptEditing(false);
            viewRouting.setSessionPromptSessionId(null);
          },
          backgroundTasksOpen: viewRouting.backgroundTasksSidebarOpen,
          onCloseBackgroundTasks: () => viewRouting.setBackgroundTasksSidebarOpen(false),
        }}
        chat={{
          sessions: sessionHooks.sessions,
          activeSessionId: sessionHooks.activeSessionId,
          onSelectSession: sessionHooks.selectSession,
          workspaces: workspaceHooks.workspaces,
          onAssignSession: workspaceHooks.assignSession,
          streamingSessionIds: chatHooks.streamingSessionIds,
          sessionWorkspaceMap: workspaceHooks.sessionWorkspaceMap,
          onNewChat: handleNewChat,
          onNewChatInWorkspace: async (workspaceId: string) => {
             const workspacePath = workspaceHooks.workspaces.find(
               (workspace) => workspace.id === workspaceId,
             )?.path;
             const newSession = await sessionHooks.createSession(undefined, {
               workspacePath: workspacePath ?? null,
             });
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
        {(viewRouting.backgroundTasksSidebarOpen || (
          viewRouting.activeView !== 'settings'
          && !(viewRouting.activeView === 'agents' && agentEditor.agentEditing)
          && !viewRouting.sessionPromptEditing
        )) && (
        <header className="main-header" data-tauri-drag-region>
          <div className="main-header-start" data-tauri-drag-region>
            {drilledInto && (
              <button
                type="button"
                className="header-back-btn"
                onClick={drillBack}
                title="Back"
                aria-label="Back"
              >
                <ChevronLeft size={18} />
              </button>
            )}
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
            <button
              className={`btn-header ${panelCtx.infoOpen ? 'active' : ''}${infoData.hasActivity ? ' has-activity' : ''}`}
              onClick={() => panelCtx.setInfoOpen(!panelCtx.infoOpen)}
              title="Info"
              id="btn-info"
            >
              <Info size={16} />
            </button>
            <WindowControls />
          </div>
        </header>
        )}

        {viewRouting.backgroundTasksSidebarOpen ? (
          <BackgroundTasksView />
        ) : viewRouting.sessionPromptEditing && viewRouting.sessionPromptSessionId ? (
          <SessionPromptPanel
            sessionId={viewRouting.sessionPromptSessionId}
            sessionTitle={
              sessionHooks.sessions.find((s) => s.id === viewRouting.sessionPromptSessionId)?.title
              ?? agentSessionHooks.sessions.find((s) => s.id === viewRouting.sessionPromptSessionId)?.title
            }
            onSaved={() => {
              sessionHooks.refreshSessions();
              agentSessionHooks.refreshSessions();
            }}
          />
        ) : (
          <>
            {viewRouting.activeView === 'chat' && <ChatView />}
            {viewRouting.activeView === 'skills' && <SkillsView />}
            {viewRouting.activeView === 'knowledge' && <KnowledgeView />}
            {viewRouting.activeView === 'agents' && <AgentsView />}
            {viewRouting.activeView === 'automation' && <AutomationView />}
            {viewRouting.activeView === 'settings' && <SettingsView />}
          </>
        )}
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
          memoryStats={obsMemoryStats}
          loading={obsLoading}
          error={obsError}
          expanded={panelCtx.obsExpanded}
          onToggleExpand={() => panelCtx.setObsExpanded(!panelCtx.obsExpanded)}
          onClose={() => panelCtx.setObsOpen(false)}
          timeRange={obsTimeRange}
          onTimeRangeChange={(r: string) => setObsTimeRange(r as TimeRange)}
        />
      )}

      {panelCtx.infoOpen && (
        <InfoPanel
          modifiedFiles={infoData.modifiedFiles}
          plans={infoData.plans}
          loopStatus={infoData.loopStatus}
          childSessions={infoData.childSessions}
          onOpenChildSession={openChildSession}
          expanded={panelCtx.infoExpanded}
          onToggleExpand={() => panelCtx.setInfoExpanded(!panelCtx.infoExpanded)}
          onClose={() => panelCtx.setInfoOpen(false)}
        />
      )}
    </div>
  );
}
