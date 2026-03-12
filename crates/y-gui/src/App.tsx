import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Sidebar } from './components/Sidebar';
import { ChatPanel } from './components/ChatPanel';
import { InputArea } from './components/InputArea';
import { StatusBar } from './components/StatusBar';
import { SettingsOverlay } from './components/SettingsOverlay';
import { useChat } from './hooks/useChat';
import { useSessions } from './hooks/useSessions';
import { useConfig } from './hooks/useConfig';
import type { SystemStatus } from './types';
import './App.css';

function App() {
  const { messages, isStreaming, error, sendMessage, loadMessages, clearMessages } = useChat();
  const {
    sessions,
    activeSessionId,
    createSession,
    selectSession,
    deleteSession,
    refreshSessions,
  } = useSessions();
  const { config, updateConfig } = useConfig();

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [lastModel, setLastModel] = useState<string | undefined>();
  const [lastTokens, setLastTokens] = useState<{ input: number; output: number } | undefined>();
  const [lastCost, setLastCost] = useState<number | undefined>();

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

  // Track last response metadata.
  useEffect(() => {
    const lastAssistant = [...messages].reverse().find((m) => m.role === 'assistant');
    if (lastAssistant) {
      setLastModel(lastAssistant.model);
      setLastTokens(lastAssistant.tokens);
      setLastCost(lastAssistant.cost);
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
      const result = await sendMessage(message, sid);
      if (result && result.session_id !== activeSessionId) {
        selectSession(result.session_id);
        refreshSessions();
      }
    },
    [activeSessionId, createSession, sendMessage, selectSession, refreshSessions],
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

  return (
    <div className="app">
      <Sidebar
        sessions={sessions}
        activeSessionId={activeSessionId}
        onSelectSession={selectSession}
        onNewChat={handleNewChat}
        onDeleteSession={handleDeleteSession}
      />

      <main className="main-panel">
        <header className="main-header">
          <h1 className="app-title">y-agent</h1>
          <button
            className="btn-settings"
            onClick={() => setSettingsOpen(true)}
            title="Settings"
          >
            ⚙️
          </button>
        </header>

        <ChatPanel messages={messages} isStreaming={isStreaming} error={error} />
        <InputArea
          onSend={handleSend}
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
        />
      </main>

      {settingsOpen && (
        <SettingsOverlay
          config={config}
          onSave={updateConfig}
          onClose={() => setSettingsOpen(false)}
        />
      )}
    </div>
  );
}

export default App;
