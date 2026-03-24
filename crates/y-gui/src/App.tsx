import { useState, useEffect, useCallback, useRef, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
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
import type { SystemStatus, ProviderInfo, TurnMeta, ChatCompletePayload } from './types';
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

  const [activeView, setActiveView] = useState<ViewType>('chat');
  const [inputExpanded, setInputExpanded] = useState(false);
  // Track which workspace is selected on the welcome page.
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
  // Show the window once the React tree is mounted to avoid the white-flash
  // that occurs when the native window renders before CSS is applied.
  useEffect(() => {
    invoke('show_window').catch(() => {});
  }, []);

  // When not in chat view, treat diagnostics as global (no active session).
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

  // Knowledge state
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

  const { agents, getAgentDetail, saveAgent, resetAgent, reloadAgents } = useAgents();

  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  /** Map from provider ID to icon identifier (loaded from providers TOML). */
  const [providerIconMap, setProviderIconMap] = useState<Record<string, string>>({});
  const [statusBarMeta, setStatusBarMeta] = useState<{
    provider?: string;
    providerId?: string;
    tokens?: { input: number; output: number };
    cost?: number;
    contextWindow?: number;
    contextTokensUsed?: number;
  }>({});

  // Reusable: fetch the latest provider list from backend.
  const refreshProviders = useCallback(() => {
    invoke<ProviderInfo[]>('provider_list')
      .then(setProviders)
      .catch(console.error);
  }, []);

  // Build provider icon map from the providers TOML config.
  const refreshProviderIcons = useCallback(() => {
    loadSection('providers')
      .then((toml) => {
        try {
          // Simple TOML parsing: extract icon = "..." lines within [[providers]] blocks.
          const map: Record<string, string> = {};
          let currentId: string | null = null;
          for (const line of toml.split('\n')) {
            const trimmed = line.trim();
            const idMatch = trimmed.match(/^id\s*=\s*"([^"]+)"/);
            if (idMatch) {
              currentId = idMatch[1];
            }
            const iconMatch = trimmed.match(/^icon\s*=\s*"([^"]+)"/);
            if (iconMatch && currentId) {
              map[currentId] = iconMatch[1];
            }
            // Reset on new provider block.
            if (trimmed === '[[providers]]') {
              currentId = null;
            }
          }
          setProviderIconMap(map);
        } catch {
          // Ignore parse errors for icon map.
        }
      })
      .catch(() => {});
  }, [loadSection]);

  // Load system status and provider list on mount.
  useEffect(() => {
    invoke<SystemStatus>('system_status')
      .then(setSystemStatus)
      .catch(console.error);
    refreshProviders();
    refreshProviderIcons();
  }, [refreshProviders, refreshProviderIcons]);

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
          providerId: meta.provider_id || undefined,
          tokens: {
            input: meta.context_tokens_used || meta.input_tokens,
            output: meta.output_tokens,
          },
          cost: meta.cost_usd,
          contextWindow: meta.context_window,
          contextTokensUsed: meta.context_tokens_used,
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

  // Listen directly to chat:complete events for status bar meta.
  // This is the authoritative source — fires once per turn completion with
  // all fields already resolved.  Avoids the race condition where the
  // messages-based useEffect would process the streaming placeholder
  // (which lacks metadata) before the backend reload finishes.
  const activeSessionIdRef = useRef(activeSessionId);
  activeSessionIdRef.current = activeSessionId;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<ChatCompletePayload>('chat:complete', (e) => {
      const payload = e.payload;
      // Only update if the event belongs to the currently viewed session.
      if (payload.session_id !== activeSessionIdRef.current) return;
      startTransition(() => {
        setStatusBarMeta({
          provider: payload.model || payload.provider_id || undefined,
          providerId: payload.provider_id || undefined,
          tokens: {
            input: payload.context_tokens_used || payload.input_tokens,
            output: payload.output_tokens,
          },
          cost: payload.cost_usd,
          contextWindow: payload.context_window,
          contextTokensUsed: payload.context_tokens_used,
        });
      });
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  // Live update: when diagnostics entries change during an active run,
  // extract the latest llm_response and update the status bar so the
  // token occupancy reflects each iteration in real time.
  useEffect(() => {
    if (!isActive) return;
    // Find the last llm_response entry from the root agent only.
    // Subagent entries (title-generator, pruning-summarizer, etc.) carry
    // a different agent_name and must not overwrite the status bar.
    for (let i = entries.length - 1; i >= 0; i--) {
      const ev = entries[i].event;
      if (ev.type === 'llm_response' && (!ev.agent_name || ev.agent_name === 'chat-turn')) {
        startTransition(() => {
          setStatusBarMeta((prev) => ({
            ...prev,
            provider: ev.model || prev.provider,
            tokens: { input: ev.input_tokens, output: ev.output_tokens },
            cost: (prev.cost ?? 0) > ev.cost_usd ? prev.cost : ev.cost_usd,
            contextTokensUsed: ev.input_tokens,
            contextWindow: ev.context_window || prev.contextWindow,
          }));
        });
        break;
      }
    }
  }, [entries, isActive]);

  // Fallback: extract status bar meta from loaded messages (session switch,
  // page reload). Only runs if there are backend-loaded messages that
  // aren't streaming placeholders.
  useEffect(() => {
    const lastAssistant = [...messages].reverse().find(
      (m) => m.role === 'assistant' && !m.id?.startsWith('streaming-'),
    );
    if (!lastAssistant) return;

    const meta = lastAssistant.metadata as Record<string, unknown> | undefined;
    const usage = meta?.usage as Record<string, unknown> | undefined;
    const providerId = (meta?.provider_id as string | undefined);
    const model = lastAssistant.model
      || (meta?.model as string | undefined)
      || providerId;
    const tokens = lastAssistant.tokens
      || (meta?.input_tokens != null && meta?.output_tokens != null
        ? { input: meta.input_tokens as number, output: meta.output_tokens as number }
        : undefined)
      || (usage?.input_tokens != null && usage?.output_tokens != null
        ? { input: usage.input_tokens as number, output: usage.output_tokens as number }
        : undefined);
    const cost = lastAssistant.cost ?? (meta?.cost_usd as number | undefined);
    const contextWindow = lastAssistant.context_window ?? (meta?.context_window as number | undefined);
    const contextTokensUsed = (meta?.context_tokens_used as number | undefined);

    if (model || tokens || cost != null || contextWindow != null) {
      setStatusBarMeta((prev) => ({
        provider: model || undefined,
        providerId: providerId || prev.providerId,
        tokens: tokens && contextTokensUsed
          ? { input: contextTokensUsed, output: tokens.output }
          : tokens,
        cost,
        contextWindow: contextWindow ?? undefined,
        contextTokensUsed: contextTokensUsed ?? undefined,
      }));
    }
  }, [messages]);

  // ------------------------------------------------------------------
  // Handlers -- thin delegation to useChat
  // ------------------------------------------------------------------

  const handleSend = useCallback(
    async (message: string, skillNames?: string[], knowledgeCollections?: string[]) => {
      let sid = activeSessionId;
      if (!sid) {
        const session = await createSession();
        if (!session) return;
        sid = session.id;

        // If a workspace is selected on the welcome page, assign the session.
        if (welcomeWorkspaceId) {
          await assignSession(welcomeWorkspaceId, sid);
        }
      }

      const providerArg = selectedProviderId === 'auto' ? undefined : selectedProviderId;

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

      // Normal send — pass skills and knowledge collections to the backend.
      addUserMessage(message, sid);
      const result = await sendMessage(message, sid, providerArg, skillNames, knowledgeCollections);
      if (result) {
        if (result.session_id !== activeSessionId) {
          selectSession(result.session_id);
        }
        refreshSessions();
      }
    },
    [activeSessionId, createSession, sendMessage, selectSession, refreshSessions, addUserMessage, selectedProviderId, pendingEdit, editAndResend, welcomeWorkspaceId, assignSession],
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

  const handleClearSession = useCallback(async () => {
    if (!activeSessionId) return;
    await deleteSession(activeSessionId);
    clearMessages();
  }, [activeSessionId, deleteSession, clearMessages]);

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
        case 'compact':
          if (activeSessionId) {
            invoke('context_compact', { sessionId: activeSessionId })
              .then(() => console.log('Compaction completed'))
              .catch((e) => console.error('Compaction failed:', e));
          }
          return true;
        case 'settings':
          setActiveView('settings');
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
          setActiveView('settings');
          return true;
        case 'export':
          // Phase 1 placeholder: log to console until export UI is built.
          console.log('Export command triggered -- not yet implemented');
          return true;
        case 'model':
          // Phase 1 placeholder: open settings on the providers tab.
          setActiveView('settings');
          return true;
        default:
          return false;
      }
    },
    [handleNewChat, clearMessages, activeSessionId],
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
        knowledgeCollections={kbCollections}
        selectedKbCollection={selectedKbCollection}
        onSelectKbCollection={(name) => { setActiveView('knowledge'); setSelectedKbCollection(name); }}
        onCreateKbCollection={createKbCollection}
        kbIngestStatus={kbIngestStatus}
        kbBatchProgress={kbBatchProgress}
        kbIngestError={kbIngestError}
        onClearKbIngestStatus={clearKbIngestStatus}
        onCancelKbIngest={cancelKbIngest}
        agents={agents}
        activeAgentId={activeAgentId}
        onSelectAgent={(id) => { setActiveView('agents'); setActiveAgentId(id); }}
        activeSettingsTab={activeSettingsTab}
        onSelectSettingsTab={(tab) => { setActiveView('settings'); setActiveSettingsTab(tab as SettingsTab); }}
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
