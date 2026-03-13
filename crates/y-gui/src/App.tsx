import { useState, useEffect, useCallback, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Settings, Activity } from 'lucide-react';
import { Sidebar } from './components/Sidebar';
import { ChatPanel } from './components/ChatPanel';
import { InputArea } from './components/InputArea';
import { StatusBar } from './components/StatusBar';
import { SettingsOverlay } from './components/SettingsOverlay';
import { DiagnosticsPanel } from './components/DiagnosticsPanel';
import { useChat } from './hooks/useChat';
import { useSessions } from './hooks/useSessions';
import { useConfig } from './hooks/useConfig';
import { useDiagnostics } from './hooks/useDiagnostics';
import { useWorkspaces } from './hooks/useWorkspaces';
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
  const { messages, isStreaming, isLoadingMessages, streamingSessionIds, error, sendMessage, cancelRun, loadMessages, clearMessages } =
    useChat(activeSessionId);

  const { config, updateConfig, loadSection, saveSection, reloadConfig } = useConfig();
  const { entries, summary, isActive, clear: clearDiagnostics, addUserMessage } =
    useDiagnostics(activeSessionId);
  const {
    workspaces,
    sessionWorkspaceMap,
    updateWorkspace,
    deleteWorkspace,
    assignSession,
    unassignSession,
    refreshWorkspaces,
  } = useWorkspaces();

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [diagOpen, setDiagOpen] = useState(false);
  const [diagExpanded, setDiagExpanded] = useState(false);
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  const [statusBarMeta, setStatusBarMeta] = useState<{
    provider?: string;
    tokens?: { input: number; output: number };
    cost?: number;
    contextWindow?: number;
  }>({});

  // Load system status and provider list on mount.
  useEffect(() => {
    invoke<SystemStatus>('system_status')
      .then(setSystemStatus)
      .catch(console.error);
    invoke<ProviderInfo[]>('provider_list')
      .then(setProviders)
      .catch(console.error);
  }, []);

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
  // Two sources:
  //   1. When session changes: query backend cache (persists across switches).
  //   2. When a new assistant message arrives in the current session: use message metadata.
  // Both paths write the same four state variables.
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

  const handleSend = useCallback(
    async (message: string) => {
      let sid = activeSessionId;
      if (!sid) {
        const session = await createSession();
        if (!session) return;
        sid = session.id;
      }
      addUserMessage(message, sid);
      const providerArg = selectedProviderId === 'auto' ? undefined : selectedProviderId;
      const result = await sendMessage(message, sid, providerArg);
      if (result) {
        if (result.session_id !== activeSessionId) {
          selectSession(result.session_id);
        }
        // Always refresh so auto-generated session titles appear immediately.
        refreshSessions();
      }
    },
    [activeSessionId, createSession, sendMessage, selectSession, refreshSessions, addUserMessage, selectedProviderId],
  );

  const handleNewChat = useCallback(async () => {
    clearMessages();
    const session = await createSession();
    if (session) {
      selectSession(session.id);
    }
  }, [createSession, selectSession, clearMessages]);

  const handleDeleteSession = useCallback(
    async (id: string) => {
      await deleteSession(id);
      if (activeSessionId === id) {
        clearMessages();
      }
    },
    [deleteSession, activeSessionId, clearMessages],
  );

  // Called from WorkspaceDialog (embedded in Sidebar) with already-chosen name + path.
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

  return (
    <div className="app">
      <Sidebar
        sessions={sessions}
        activeSessionId={activeSessionId}
        streamingSessionIds={streamingSessionIds}
        workspaces={workspaces}
        sessionWorkspaceMap={sessionWorkspaceMap}
        onSelectSession={selectSession}
        onNewChat={handleNewChat}
        onDeleteSession={handleDeleteSession}
        onCreateWorkspace={handleCreateWorkspace}
        onUpdateWorkspace={updateWorkspace}
        onDeleteWorkspace={deleteWorkspace}
        onAssignSession={assignSession}
        onUnassignSession={unassignSession}
      />

      <main className="main-panel">
        <header className="main-header">
          <h1 className="app-title">y-agent</h1>
          <div className="header-actions">
            <button
              className={`btn-header ${diagOpen ? 'active' : ''} ${isActive ? 'has-activity' : ''}`}
              onClick={() => setDiagOpen(!diagOpen)}
              title="Diagnostics"
              id="btn-diagnostics"
            >
              <Activity size={16} />
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

        <ChatPanel messages={messages} isStreaming={isStreaming} isLoading={isLoadingMessages} error={error} />
        <InputArea
          onSend={handleSend}
          onStop={cancelRun}
          disabled={isStreaming}
          sendOnEnter={config.send_on_enter}
          providers={providers}
          selectedProviderId={selectedProviderId}
          onSelectProvider={setSelectedProviderId}
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
      </main>

      {diagOpen && (
        <DiagnosticsPanel
          entries={entries}
          summary={summary}
          isActive={isActive}
          expanded={diagExpanded}
          onToggleExpand={() => setDiagExpanded(!diagExpanded)}
          onClear={clearDiagnostics}
          onClose={() => {
            setDiagOpen(false);
            setDiagExpanded(false);
          }}
        />
      )}

      {settingsOpen && (
        <SettingsOverlay
          config={config}
          onSave={updateConfig}
          onClose={() => setSettingsOpen(false)}
          loadSection={loadSection}
          saveSection={saveSection}
          reloadConfig={reloadConfig}
        />
      )}
    </div>
  );
}

export default App;
