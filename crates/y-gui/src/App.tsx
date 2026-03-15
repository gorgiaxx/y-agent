import { useState, useEffect, useCallback, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Settings, Activity, Eye } from 'lucide-react';
import { Sidebar } from './components/Sidebar';
import type { ViewType } from './components/Sidebar';
import { ChatPanel } from './components/ChatPanel';
import { InputArea } from './components/InputArea';
import { StatusBar } from './components/StatusBar';
import { SettingsOverlay } from './components/SettingsOverlay';
import { DiagnosticsPanel } from './components/DiagnosticsPanel';
import { ObservabilityPanel } from './components/ObservabilityPanel';
import { SkillsPanel } from './components/SkillsPanel';
import { SkillImportDialog } from './components/SkillImportDialog';
import { useChat } from './hooks/useChat';
import { useSessions } from './hooks/useSessions';
import { useConfig } from './hooks/useConfig';
import { useDiagnostics } from './hooks/useDiagnostics';
import { useObservability } from './hooks/useObservability';
import { useWorkspaces } from './hooks/useWorkspaces';
import { useSkills } from './hooks/useSkills';
import { useThemeProvider, ThemeContext } from './hooks/useTheme';
import type { SystemStatus, ProviderInfo, TurnMeta } from './types';
import './App.css';

function App() {
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

  const [activeView, setActiveView] = useState<ViewType>('chat');
  // When not in chat view, treat diagnostics as global (no active session).
  const diagnosticSessionId = activeView === 'chat' ? activeSessionId : null;
  const { entries, summary, isActive, clear: clearDiagnostics, addUserMessage } =
    useDiagnostics(diagnosticSessionId);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [diagOpen, setDiagOpen] = useState(false);
  const [obsOpen, setObsOpen] = useState(false);
  const [obsExpanded, setObsExpanded] = useState(false);
  const [diagExpanded, setDiagExpanded] = useState(false);
  const { snapshot: obsSnapshot, loading: obsLoading } = useObservability(obsOpen);
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
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  const [statusBarMeta, setStatusBarMeta] = useState<{
    provider?: string;
    tokens?: { input: number; output: number };
    cost?: number;
    contextWindow?: number;
  }>({});

  // Reusable: fetch the latest provider list from backend.
  const refreshProviders = useCallback(() => {
    invoke<ProviderInfo[]>('provider_list')
      .then(setProviders)
      .catch(console.error);
  }, []);

  // Load system status and provider list on mount.
  useEffect(() => {
    invoke<SystemStatus>('system_status')
      .then(setSystemStatus)
      .catch(console.error);
    refreshProviders();
  }, [refreshProviders]);

  // Developer mode: Ctrl+Shift+I (or Cmd+Shift+I on macOS) toggles DevTools.
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

  // Load messages when active session changes.
  useEffect(() => {
    if (activeSessionId) {
      loadMessages(activeSessionId);
    } else {
      clearMessages();
    }
  }, [activeSessionId, loadMessages, clearMessages]);

  // Track last response metadata for status bar.
  const applyMeta = useCallback((meta: TurnMeta | null) => {
    startTransition(() => {
      if (meta) {
        setStatusBarMeta({
          provider: meta.model || meta.provider_id || undefined,
          tokens: { input: meta.input_tokens, output: meta.output_tokens },
          cost: meta.cost_usd,
          contextWindow: meta.context_window,
        });
      } else {
        setStatusBarMeta({});
      }
    });
  }, []);

  // On session switch: restore from backend-cached metadata.
  useEffect(() => {
    if (!activeSessionId) {
      applyMeta(null);
      return;
    }
    invoke<TurnMeta | null>('session_last_turn_meta', { sessionId: activeSessionId })
      .then(applyMeta)
      .catch(() => applyMeta(null));
  }, [activeSessionId, applyMeta]);

  // When a new assistant message arrives (real-time): update from message metadata.
  useEffect(() => {
    const lastAssistant = [...messages].reverse().find((m) => m.role === 'assistant');
    if (lastAssistant && lastAssistant.model) {
      setStatusBarMeta({
        provider: lastAssistant.model || lastAssistant.provider_id || undefined,
        tokens: lastAssistant.tokens,
        cost: lastAssistant.cost,
        contextWindow: lastAssistant.context_window,
      });
    }
  }, [messages]);

  // ------------------------------------------------------------------
  // Handlers -- thin delegation to useChat
  // ------------------------------------------------------------------

  const handleSend = useCallback(
    async (message: string, skillNames?: string[]) => {
      let sid = activeSessionId;
      if (!sid) {
        const session = await createSession();
        if (!session) return;
        sid = session.id;
      }

      const providerArg = selectedProviderId === 'auto' ? undefined : selectedProviderId;

      // TODO: pass skillNames to the backend when skill-aware chat is implemented.
      if (skillNames && skillNames.length > 0) {
        console.log('Skills attached to message:', skillNames);
      }

      // If in edit mode, use the transactional editAndResend.
      if (pendingEdit) {
        addUserMessage(message, sid);
        const result = await editAndResend(sid, message, providerArg);
        if (result) {
          if (result.session_id !== activeSessionId) {
            selectSession(result.session_id);
          }
          refreshSessions();
        }
        return;
      }

      // Normal send.
      addUserMessage(message, sid);
      const result = await sendMessage(message, sid, providerArg);
      if (result) {
        if (result.session_id !== activeSessionId) {
          selectSession(result.session_id);
        }
        refreshSessions();
      }
    },
    [activeSessionId, createSession, sendMessage, selectSession, refreshSessions, addUserMessage, selectedProviderId, pendingEdit, editAndResend],
  );

  const handleEditMessage = useCallback((content: string, messageId: string) => {
    editMessage(messageId, content);
  }, [editMessage]);

  const handleUndoMessage = useCallback(
    async (messageId: string) => {
      if (!activeSessionId) return;
      await undoToMessage(activeSessionId, messageId);
    },
    [activeSessionId, undoToMessage],
  );

  const handleCancelEdit = useCallback(() => {
    cancelEdit();
  }, [cancelEdit]);

  const handleRestoreBranch = useCallback(
    async (checkpointId: string) => {
      if (!activeSessionId) return;
      await restoreBranch(activeSessionId, checkpointId);
    },
    [activeSessionId, restoreBranch],
  );

  const handleResendMessage = useCallback(
    async (content: string, messageId: string) => {
      if (!activeSessionId) return;
      const providerArg = selectedProviderId === 'auto' ? undefined : selectedProviderId;
      await resendLastTurn(activeSessionId, messageId, content, providerArg);
    },
    [activeSessionId, resendLastTurn, selectedProviderId],
  );

  const handleNewChat = useCallback(async () => {
    clearMessages();
    const session = await createSession();
    if (session) {
      selectSession(session.id);
    }
  }, [createSession, selectSession, clearMessages]);

  const handleNewChatInWorkspace = useCallback(
    async (workspaceId: string) => {
      clearMessages();
      const session = await createSession();
      if (session) {
        await assignSession(workspaceId, session.id);
        selectSession(session.id);
      }
    },
    [createSession, selectSession, clearMessages, assignSession],
  );

  const handleDeleteSession = useCallback(
    async (id: string) => {
      await deleteSession(id);
      if (activeSessionId === id) {
        clearMessages();
      }
    },
    [deleteSession, activeSessionId, clearMessages],
  );

  const handleCreateWorkspace = useCallback(
    async (name: string, path: string) => {
      try {
        await invoke('workspace_create', { name, path });
        await refreshWorkspaces();
      } catch (e) {
        console.error('Failed to create workspace:', e);
      }
    },
    [refreshWorkspaces],
  );

  // ------------------------------------------------------------------
  // Slash-command handler -- maps command names to existing GUI actions
  // ------------------------------------------------------------------

  const handleCommand = useCallback(
    (commandName: string): boolean => {
      switch (commandName) {
        case 'new':
          handleNewChat();
          return true;
        case 'clear':
          clearMessages();
          return true;
        case 'settings':
          setSettingsOpen(true);
          return true;
        case 'diagnostics':
          setDiagOpen((prev) => !prev);
          return true;
        case 'observability':
          setObsOpen((prev) => !prev);
          return true;
        case 'status':
          invoke<SystemStatus>('system_status')
            .then(setSystemStatus)
            .catch(console.error);
          return true;
        case 'help':
          // Phase 1 placeholder: open settings as a stand-in.
          setSettingsOpen(true);
          return true;
        case 'export':
          // Phase 1 placeholder: log to console until export UI is built.
          console.log('Export command triggered -- not yet implemented');
          return true;
        case 'model':
          // Phase 1 placeholder: open settings on the providers tab.
          setSettingsOpen(true);
          return true;
        default:
          return false;
      }
    },
    [handleNewChat, clearMessages],
  );

  // Determine if input should be disabled: streaming OR a compound operation is in progress.
  const inputDisabled = isStreaming || (opStatus !== 'idle' && opStatus !== 'sending');

  return (
    <ThemeContext.Provider value={themeCtx}>
    <div className="app">
      <Sidebar
        sessions={sessions}
        activeSessionId={activeSessionId}
        streamingSessionIds={streamingSessionIds}
        workspaces={workspaces}
        sessionWorkspaceMap={sessionWorkspaceMap}
        activeView={activeView}
        skills={skills}
        activeSkillName={activeSkillName}
        importStatus={importStatus}
        importError={importError}
        onSelectView={setActiveView}
        onSelectSession={(id) => { setActiveView('chat'); selectSession(id); }}
        onSelectSkill={(name) => { setActiveView('skills'); setActiveSkillName(name); }}
        onImportClick={() => setImportDialogOpen(true)}
        onClearImportStatus={clearImportStatus}
        onNewChat={() => { setActiveView('chat'); handleNewChat(); }}
        onNewChatInWorkspace={(wsId) => { setActiveView('chat'); handleNewChatInWorkspace(wsId); }}
        onDeleteSession={handleDeleteSession}
        onCreateWorkspace={handleCreateWorkspace}
        onUpdateWorkspace={updateWorkspace}
        onDeleteWorkspace={deleteWorkspace}
        onAssignSession={assignSession}
        onUnassignSession={unassignSession}
      />

      <main className="main-panel">
        <header className="main-header">
          <h1 className="app-title">
            {activeView === 'skills'
              ? 'Skills'
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
            <button
              className="btn-header"
              onClick={() => setSettingsOpen(true)}
              title="Settings"
              id="btn-settings"
            >
              <Settings size={16} />
            </button>
          </div>
        </header>

        {activeView === 'chat' && (
          <>
            <ChatPanel messages={messages} isStreaming={isStreaming} isLoading={isLoadingMessages} error={error} onEditMessage={handleEditMessage} onUndoMessage={handleUndoMessage} onResendMessage={handleResendMessage} onRestoreBranch={handleRestoreBranch} toolResults={toolResults} />
            <InputArea
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
            />
            <StatusBar
              providerCount={systemStatus?.provider_count ?? 0}
              sessionCount={systemStatus?.session_count ?? null}
              version={systemStatus?.version ?? '0.1.0'}
              activeModel={statusBarMeta.provider}
              lastTokens={statusBarMeta.tokens}
              lastCost={statusBarMeta.cost}
              contextWindow={statusBarMeta.contextWindow}
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
      </main>

      {diagOpen && (
        <DiagnosticsPanel
          entries={entries}
          summary={summary}
          isActive={isActive}
          isGlobal={!diagnosticSessionId}
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
          expanded={obsExpanded}
          onToggleExpand={() => setObsExpanded(!obsExpanded)}
          onClose={() => {
            setObsOpen(false);
            setObsExpanded(false);
          }}
        />
      )}

      {settingsOpen && (
        <SettingsOverlay
          config={config}
          onSave={(updates) => {
            updateConfig(updates);
            // Refresh the provider dropdown after settings are saved.
            refreshProviders();
          }}
          onClose={() => setSettingsOpen(false)}
          loadSection={loadSection}
          saveSection={saveSection}
          reloadConfig={async () => {
            const msg = await rawReloadConfig();
            // After hot-reloading config, refresh provider list too.
            refreshProviders();
            return msg;
          }}
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
    </div>
    </ThemeContext.Provider>
  );
}

export default App;
