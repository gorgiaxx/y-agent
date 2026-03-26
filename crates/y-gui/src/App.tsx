import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Activity, Eye } from 'lucide-react';
import { Sidebar } from './components/Sidebar';
import type { ViewType } from './components/Sidebar';
import { ChatPanel } from './components/chat-panel/ChatPanel';
import { WelcomePage } from './components/WelcomePage';
import { InputArea } from './components/chat-panel/input-area/InputArea';
import { StatusBar } from './components/chat-panel/StatusBar';
import { SettingsPanel } from './components/settings/SettingsPanel';
import type { SettingsTab } from './components/settings/SettingsPanel';
import { DiagnosticsPanel } from './components/observation/DiagnosticsPanel';
import { ObservabilityPanel } from './components/observation/ObservabilityPanel';
import { SkillsPanel } from './components/skills/SkillsPanel';
import { KnowledgePanel } from './components/knowledge/KnowledgePanel';
import { AgentsPanel } from './components/agents/AgentsPanel';
import { SkillImportDialog } from './components/skills/SkillImportDialog';
import { WorkspaceDialog } from './components/chat-panel/WorkspaceDialog';
import { useChat } from './hooks/useChat';
import { useSessions } from './hooks/useSessions';
import { useConfig } from './hooks/useConfig';
import { useDiagnostics } from './hooks/useDiagnostics';
import { useObservability } from './hooks/useObservability';
import type { TimeRange } from './hooks/useObservability';
import { useWorkspaces } from './hooks/useWorkspaces';
import { useSkills } from './hooks/useSkills';
import { useKnowledge } from './hooks/useKnowledge';
import { useAgents } from './hooks/useAgents';
import { useThemeProvider, ThemeContext } from './hooks/useTheme';
import { useProviders } from './hooks/useProviders';
import { useStatusBarMeta } from './hooks/useStatusBarMeta';
import { useChatHandlers } from './hooks/useChatHandlers';
import './App.css';

function App() {
  // ------------------------------------------------------------------
  // Domain hooks
  // ------------------------------------------------------------------

  const {
    sessions,
    activeSessionId,
    createSession,
    selectSession,
    deleteSession,
    refreshSessions,
  } = useSessions();

  const {
    messages,
    isStreaming,
    isLoadingMessages,
    streamingSessionIds,
    error,
    opStatus,
    pendingEdit,
    sendMessage,
    cancelRun,
    loadMessages,
    clearMessages,
    editMessage,
    cancelEdit,
    editAndResend,
    undoToMessage,
    resendLastTurn,
    restoreBranch,
    toolResults,
    contextResetPoints,
    addContextReset,
  } = useChat(activeSessionId);

  const { config, updateConfig, loadSection, saveSection, reloadConfig: rawReloadConfig } = useConfig();
  const themeCtx = useThemeProvider(config.theme);

  const {
    workspaces,
    sessionWorkspaceMap,
    updateWorkspace,
    deleteWorkspace,
    assignSession,
    unassignSession,
    refreshWorkspaces,
  } = useWorkspaces();

  // ------------------------------------------------------------------
  // Navigation state
  // ------------------------------------------------------------------

  const [activeView, setActiveView] = useState<ViewType>('chat');
  const [inputExpanded, setInputExpanded] = useState(false);
  const [welcomeWorkspaceId, setWelcomeWorkspaceId] = useState<string | null>(null);

  // Default welcome workspace to first workspace (alphabetically).
  useEffect(() => {
    if (workspaces.length > 0 && !welcomeWorkspaceId) {
      const sorted = [...workspaces].sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
      );
      setWelcomeWorkspaceId(sorted[0].id);
    }
  }, [workspaces, welcomeWorkspaceId]);

  // Show the window once the React tree is mounted to avoid white-flash.
  useEffect(() => {
    invoke('show_window').catch(() => {});
  }, []);

  // ------------------------------------------------------------------
  // Diagnostics & Observability
  // ------------------------------------------------------------------

  const diagnosticSessionId = activeView === 'chat' ? activeSessionId : null;
  const { entries, summary, isActive, clear: clearDiagnostics, addUserMessage } =
    useDiagnostics(diagnosticSessionId);

  const [activeSettingsTab, setActiveSettingsTab] = useState<SettingsTab>('general');
  const [diagOpen, setDiagOpen] = useState(false);
  const [obsOpen, setObsOpen] = useState(false);
  const [obsExpanded, setObsExpanded] = useState(false);
  const [diagExpanded, setDiagExpanded] = useState(false);
  const [obsTimeRange, setObsTimeRange] = useState<TimeRange>('all');
  const { snapshot: obsSnapshot, loading: obsLoading, error: obsError } = useObservability({
    active: obsOpen,
    timeRange: obsTimeRange,
  });

  // ------------------------------------------------------------------
  // Skills
  // ------------------------------------------------------------------

  const {
    skills,
    getSkillDetail,
    uninstallSkill,
    setEnabled: setSkillEnabled,
    openFolder: openSkillFolder,
    importSkill,
    importStatus,
    importError,
    clearImportStatus,
    getSkillFiles,
    readSkillFile,
    saveSkillFile,
  } = useSkills();

  // Auto-clear success status after 2 seconds.
  useEffect(() => {
    if (importStatus === 'success') {
      const timer = setTimeout(clearImportStatus, 2000);
      return () => clearTimeout(timer);
    }
  }, [importStatus, clearImportStatus]);

  const [activeSkillName, setActiveSkillName] = useState<string | null>(null);
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null);
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [wsDialogOpen, setWsDialogOpen] = useState(false);

  // ------------------------------------------------------------------
  // Knowledge
  // ------------------------------------------------------------------

  const {
    collections: kbCollections,
    entries: kbEntries,
    selectedCollection: selectedKbCollection,
    setSelectedCollection: setSelectedKbCollection,
    createCollection: createKbCollection,
    deleteCollection: deleteKbCollection,
    renameCollection: renameKbCollection,
    getEntryDetail: getKbEntryDetail,
    deleteEntry: deleteKbEntry,
    search: kbSearch,
    ingestBatch: kbIngestBatch,
    ingestStatus: kbIngestStatus,
    ingestError: kbIngestError,
    batchProgress: kbBatchProgress,
    clearIngestStatus: clearKbIngestStatus,
    cancelIngest: cancelKbIngest,
  } = useKnowledge();

  // ------------------------------------------------------------------
  // Agents
  // ------------------------------------------------------------------

  const { agents, getAgentDetail, saveAgent, resetAgent, reloadAgents } = useAgents();

  // ------------------------------------------------------------------
  // Providers & System Status (extracted hook)
  // ------------------------------------------------------------------

  const {
    systemStatus,
    providers,
    selectedProviderId,
    setSelectedProviderId,
    providerIconMap,
    refreshProviders,
    refreshProviderIcons,
  } = useProviders(loadSection);

  // ------------------------------------------------------------------
  // Developer mode: Ctrl+Shift+I toggles DevTools
  // ------------------------------------------------------------------

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'I') {
        e.preventDefault();
        invoke('toggle_devtools').catch(console.error);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  // ------------------------------------------------------------------
  // Load messages when active session changes
  // ------------------------------------------------------------------

  useEffect(() => {
    if (activeSessionId) {
      loadMessages(activeSessionId);
    } else {
      clearMessages();
    }
  }, [activeSessionId, loadMessages, clearMessages]);

  // ------------------------------------------------------------------
  // Status bar meta (extracted hook)
  // ------------------------------------------------------------------

  const statusBarMeta = useStatusBarMeta({
    activeSessionId,
    messages,
    isStreaming,
    isLoadingMessages,
    diagnosticEntries: entries,
    isDiagnosticsActive: isActive,
  });

  // ------------------------------------------------------------------
  // Chat handlers (extracted hook)
  // ------------------------------------------------------------------

  const {
    handleSend,
    handleEditMessage,
    handleUndoMessage,
    handleCancelEdit,
    handleRestoreBranch,
    handleResendMessage,
    handleClearSession,
    handleNewChat,
    handleNewChatInWorkspace,
    handleDeleteSession,
    handleCreateWorkspace,
    handleCommand,
  } = useChatHandlers({
    activeSessionId,
    createSession,
    selectSession,
    deleteSession,
    refreshSessions,
    clearMessages,
    sendMessage,
    editAndResend,
    editMessage,
    cancelEdit,
    undoToMessage,
    resendLastTurn,
    restoreBranch,
    pendingEdit,
    selectedProviderId,
    welcomeWorkspaceId,
    assignSession,
    refreshWorkspaces,
    addUserMessage,
    setActiveView,
    setDiagOpen,
    setObsOpen,
  });

  // Determine if input should be disabled: streaming OR a compound operation is in progress.
  const inputDisabled = isStreaming || (opStatus !== 'idle' && opStatus !== 'sending');

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  return (
    <ThemeContext.Provider value={themeCtx}>
    <div className="app">
      <Sidebar
        chat={{
          sessions,
          activeSessionId,
          streamingSessionIds,
          workspaces,
          sessionWorkspaceMap,
          onSelectSession: (id) => { setActiveView('chat'); selectSession(id); },
          onNewChat: () => { setActiveView('chat'); handleNewChat(); },
          onNewChatInWorkspace: (wsId) => { setActiveView('chat'); handleNewChatInWorkspace(wsId); },
          onDeleteSession: handleDeleteSession,
          onCreateWorkspace: handleCreateWorkspace,
          onUpdateWorkspace: updateWorkspace,
          onDeleteWorkspace: deleteWorkspace,
          onAssignSession: assignSession,
          onUnassignSession: unassignSession,
        }}
        skills={{
          skills,
          activeSkillName,
          importStatus,
          importError,
          onSelectSkill: (name) => { setActiveView('skills'); setActiveSkillName(name); },
          onImportClick: () => setImportDialogOpen(true),
          onClearImportStatus: clearImportStatus,
        }}
        knowledge={{
          collections: kbCollections,
          selectedCollection: selectedKbCollection,
          onSelectCollection: (name) => { setActiveView('knowledge'); setSelectedKbCollection(name); },
          onCreateCollection: createKbCollection,
          ingestStatus: kbIngestStatus,
          batchProgress: kbBatchProgress,
          ingestError: kbIngestError,
          onClearIngestStatus: clearKbIngestStatus,
          onCancelIngest: cancelKbIngest,
        }}
        agents={{
          agents,
          activeAgentId,
          onSelectAgent: (id) => { setActiveView('agents'); setActiveAgentId(id); },
        }}
        nav={{
          activeView,
          onSelectView: setActiveView,
          activeSettingsTab,
          onSelectSettingsTab: (tab) => { setActiveView('settings'); setActiveSettingsTab(tab as SettingsTab); },
        }}
      />

      <main className="main-panel">
        {activeView !== 'settings' && (
        <header className="main-header">
          <h1 className="app-title">
            {activeView === 'skills'
              ? 'Skills'
              : activeView === 'knowledge'
                ? 'Knowledge'
              : activeView === 'agents'
              ? 'Agents'
              : activeSessionId
                ? sessions.find((s) => s.id === activeSessionId)?.title || 'Untitled'
                : 'y-agent'}
          </h1>
          <div className="header-actions">
            <button
              className={`btn-header ${diagOpen ? 'active' : ''}`}
              onClick={() => setDiagOpen(!diagOpen)}
              title="Diagnostics"
              id="btn-diagnostics"
            >
              <Activity size={16} />
            </button>
            <button
              className={`btn-header ${obsOpen ? 'active' : ''}`}
              onClick={() => setObsOpen(!obsOpen)}
              title="Observability"
              id="btn-observability"
            >
              <Eye size={16} />
            </button>
          </div>
        </header>
        )}

        {activeView === 'chat' && (
          <>
            {!inputExpanded && activeSessionId && (
              <ChatPanel messages={messages} isStreaming={isStreaming} isLoading={isLoadingMessages} error={error} onEditMessage={handleEditMessage} onUndoMessage={handleUndoMessage} onResendMessage={handleResendMessage} onRestoreBranch={handleRestoreBranch} toolResults={toolResults} contextResetPoints={contextResetPoints} />
            )}
            {!inputExpanded && !activeSessionId && (
              <WelcomePage
                workspaces={workspaces}
                selectedWorkspaceId={welcomeWorkspaceId}
                onSelectWorkspace={setWelcomeWorkspaceId}
                onCreateWorkspace={() => setWsDialogOpen(true)}
              />
            )}
            <InputArea
              key={activeSessionId ?? '__no_session__'}
              onSend={handleSend}
              onStop={cancelRun}
              onCommand={handleCommand}
              disabled={inputDisabled}
              sendOnEnter={config.send_on_enter}
              providers={providers}
              selectedProviderId={selectedProviderId}
              onSelectProvider={setSelectedProviderId}
              pendingEdit={pendingEdit}
              onCancelEdit={handleCancelEdit}
              skills={skills.filter((s) => s.enabled)}
              knowledgeCollections={kbCollections}
              expanded={inputExpanded}
              onExpandChange={setInputExpanded}
              onClearSession={handleClearSession}
              onAddContextReset={addContextReset}
              providerIcons={providerIconMap}
            />
            <StatusBar
              providerCount={systemStatus?.provider_count ?? 0}
              sessionCount={systemStatus?.session_count ?? null}
              version={systemStatus?.version ?? 'debug'}
              activeModel={statusBarMeta.provider}
              activeProviderIcon={
                // Look up by provider ID first, then fall back to selected provider ID.
                (statusBarMeta.providerId ? providerIconMap[statusBarMeta.providerId] : undefined)
                ?? (selectedProviderId !== 'auto' ? providerIconMap[selectedProviderId] : undefined)
                ?? null
              }
              lastTokens={statusBarMeta.tokens}
              lastCost={statusBarMeta.cost}
              contextWindow={statusBarMeta.contextWindow}
              contextTokensUsed={statusBarMeta.contextTokensUsed}
            />
          </>
        )}

        {activeView === 'skills' && (
          <SkillsPanel
            skillName={activeSkillName}
            onGetDetail={getSkillDetail}
            onGetFiles={getSkillFiles}
            onReadFile={readSkillFile}
            onSaveFile={saveSkillFile}
            onUninstall={async (name) => {
              await uninstallSkill(name);
              setActiveSkillName(null);
            }}
            onSetEnabled={async (name, enabled) => {
              await setSkillEnabled(name, enabled);
            }}
            onOpenFolder={openSkillFolder}
          />
        )}

        {activeView === 'knowledge' && (
          <KnowledgePanel
            collections={kbCollections}
            entries={kbEntries}
            selectedCollection={selectedKbCollection}
            onSelectCollection={setSelectedKbCollection}
            onCreateCollection={createKbCollection}
            onDeleteCollection={deleteKbCollection}
            onRenameCollection={renameKbCollection}
            onGetEntryDetail={getKbEntryDetail}
            onDeleteEntry={deleteKbEntry}
            onSearch={kbSearch}
            onIngestBatch={kbIngestBatch}
          />
        )}

        {activeView === 'agents' && (
          <AgentsPanel
            agentId={activeAgentId}
            onGetDetail={getAgentDetail}
            onSave={saveAgent}
            onReset={resetAgent}
            onReload={reloadAgents}
          />
        )}

        {activeView === 'settings' && (
          <SettingsPanel
            config={config}
            activeTab={activeSettingsTab}
            onSave={(updates) => {
              updateConfig(updates);
              refreshProviders();
              refreshProviderIcons();
            }}
            loadSection={loadSection}
            saveSection={saveSection}
            reloadConfig={async () => {
              const msg = await rawReloadConfig();
              refreshProviders();
              refreshProviderIcons();
              return msg;
            }}
          />
        )}
      </main>

      {diagOpen && (
        <DiagnosticsPanel
          entries={entries}
          summary={summary}
          isActive={isActive}
          isGlobal={!diagnosticSessionId}
          sessionId={diagnosticSessionId}
          expanded={diagExpanded}
          onToggleExpand={() => setDiagExpanded(!diagExpanded)}
          onClear={clearDiagnostics}
          onClose={() => {
            setDiagOpen(false);
            setDiagExpanded(false);
          }}
        />
      )}

      {obsOpen && (
        <ObservabilityPanel
          snapshot={obsSnapshot}
          loading={obsLoading}
          error={obsError}
          expanded={obsExpanded}
          onToggleExpand={() => setObsExpanded(!obsExpanded)}
          onClose={() => {
            setObsOpen(false);
            setObsExpanded(false);
          }}
          timeRange={obsTimeRange}
          onTimeRangeChange={setObsTimeRange}
          providerIcons={providerIconMap}
        />
      )}

      {importDialogOpen && (
        <SkillImportDialog
          onImport={(path, sanitize) => {
            importSkill(path, sanitize);
          }}
          onClose={() => setImportDialogOpen(false)}
        />
      )}

      {wsDialogOpen && (
        <WorkspaceDialog
          onConfirm={(name, path) => {
            handleCreateWorkspace(name, path);
            setWsDialogOpen(false);
          }}
          onClose={() => setWsDialogOpen(false)}
        />
      )}
    </div>
    </ThemeContext.Provider>
  );
}

export default App;
