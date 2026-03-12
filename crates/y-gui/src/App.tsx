import { useState, useEffect, useCallback } from 'react';
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
import type { SystemStatus } from './types';
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
  const { messages, isStreaming, streamingSessionIds, error, sendMessage, cancelRun, loadMessages, clearMessages } =
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
  const [lastModel, setLastModel] = useState<string | undefined>();
  const [lastTokens, setLastTokens] = useState<{ input: number; output: number } | undefined>();
  const [lastCost, setLastCost] = useState<number | undefined>();
  const [lastContextWindow, setLastContextWindow] = useState<number | undefined>();

  // Load system status on mount.
  useEffect(() => {
    invoke<SystemStatus>('system_status')
      .then(setSystemStatus)
      .catch(console.error);
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
  useEffect(() => {
    const lastAssistant = [...messages].reverse().find((m) => m.role === 'assistant');
    if (lastAssistant) {
      setLastModel(lastAssistant.model);
      setLastTokens(lastAssistant.tokens);
      setLastCost(lastAssistant.cost);
      setLastContextWindow(lastAssistant.context_window);
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
      const result = await sendMessage(message, sid);
      if (result) {
        if (result.session_id !== activeSessionId) {
          selectSession(result.session_id);
        }
        // Always refresh so auto-generated session titles appear immediately.
        refreshSessions();
      }
    },
    [activeSessionId, createSession, sendMessage, selectSession, refreshSessions, addUserMessage],
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

        <ChatPanel messages={messages} isStreaming={isStreaming} error={error} />
        <InputArea
          onSend={handleSend}
          onStop={cancelRun}
          disabled={isStreaming}
          sendOnEnter={config.send_on_enter}
        />
        <StatusBar
          providerCount={systemStatus?.provider_count ?? 0}
          sessionCount={systemStatus?.session_count ?? null}
          version={systemStatus?.version ?? '0.1.0'}
          activeModel={lastModel}
          lastTokens={lastTokens}
          lastCost={lastCost}
          contextWindow={lastContextWindow}
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
