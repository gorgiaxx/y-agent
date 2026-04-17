import { useCallback, useEffect, useMemo, useState } from 'react';
import { AgentEditorPanel } from '../components/agents/AgentEditorPanel';
import { AgentOverview } from '../components/agents/AgentOverview';
import { AgentStudio } from '../components/agents/AgentStudio';
import {
  useNavigationContext,
  useAgentsContext,
  useProvidersContext,
  useConfigContext,
  useSkillsContext,
  useKnowledgeContext,
  useWorkspacesContext,
  useAgentSessionsContext,
} from '../providers/AppContexts';
import { useChat } from '../hooks/useChat';
import { useChatHandlers } from '../hooks/useChatHandlers';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useSessionInteractions } from '../hooks/useSessionInteractions';
import { useStatusBarMeta } from '../hooks/useStatusBarMeta';
import { useAgentEditor } from '../hooks/useAgentEditor';
import type { AgentDetail } from '../hooks/useAgents';
import type { PlanMode, ThinkingEffort, McpMode } from '../types';
import { useMcpServers } from '../hooks/useMcpServers';
import './AgentsView.css';

export function AgentsView() {
  const nav = useNavigationContext();
  const {
    agents,
    tools,
    promptSections,
    getAgentDetail,
    getAgentSource,
    parseAgentToml,
    saveAgent,
    resetAgent,
    reloadAgents,
  } = useAgentsContext();
  const providerHooks = useProvidersContext();
  const configHooks = useConfigContext();
  const skillHooks = useSkillsContext();
  const knowledgeHooks = useKnowledgeContext();
  const workspaceHooks = useWorkspacesContext();
  const sessionHooks = useAgentSessionsContext();
  const {
    activeSessionId: agentActiveSessionId,
    sessions: agentSessions,
    selectSession: selectAgentSession,
    refreshSessions: refreshAgentSessions,
  } = sessionHooks;
  const agentRootNames = useMemo(
    () => (nav.activeAgentId ? [nav.activeAgentId] : ['chat-turn']),
    [nav.activeAgentId],
  );
  const agentChatHooks = useChat(agentActiveSessionId, agentRootNames);
  const { loadMessages, clearMessages } = agentChatHooks;

  const [selectedAgentDetail, setSelectedAgentDetail] = useState<AgentDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort | null>(null);
  const [planMode, setPlanMode] = useState<PlanMode>('fast');
  const [rewindDraft, setRewindDraft] = useState<string | null>(null);
  const [agentQuery, setAgentQuery] = useState('');
  const [reloadingAgents, setReloadingAgents] = useState(false);

  const [mcpModeBySession, setMcpModeBySession] = useState<Record<string, McpMode>>({});
  const [mcpServersBySession, setMcpServersBySession] = useState<Record<string, string[]>>({});
  const mcpSessionKey = agentActiveSessionId ?? '__no_session__';
  const mcpMode: McpMode = mcpModeBySession[mcpSessionKey] ?? 'disabled';
  const selectedMcpServers = mcpServersBySession[mcpSessionKey] ?? [];
  const { servers: mcpServers } = useMcpServers();
  const mcpServerList = mcpServers.map((s) => ({ name: s.name, disabled: s.disabled }));
  const handleMcpModeChange = useCallback((mode: McpMode) => {
    setMcpModeBySession((prev) => ({ ...prev, [mcpSessionKey]: mode }));
  }, [mcpSessionKey]);
  const handleMcpServerToggle = useCallback((name: string) => {
    setMcpServersBySession((prev) => {
      const existing = prev[mcpSessionKey] ?? [];
      const next = existing.includes(name)
        ? existing.filter((n) => n !== name)
        : [...existing, name];
      return { ...prev, [mcpSessionKey]: next };
    });
  }, [mcpSessionKey]);

  // Agent editor -- all editor state and logic extracted to a dedicated hook
  const editor = useAgentEditor({
    getAgentDetail,
    getAgentSource,
    parseAgentToml,
    saveAgent,
    resetAgent,
    editorOpen: nav.agentEditing,
    setEditorOpen: nav.setAgentEditing,
    editorTab: nav.agentEditorTab,
    setEditorTab: nav.setAgentEditorTab,
    editorSurface: nav.agentEditorSurface,
    setEditorSurface: nav.setAgentEditorSurface,
  });

  // Register the editor's surface change handler so the sidebar toggle
  // goes through TOML validation before switching raw <-> form.
  useEffect(() => {
    nav.setAgentEditorSurfaceHandler(
      nav.agentEditing ? (surface) => { void editor.handleEditorSurfaceChange(surface); } : null,
    );
    return () => nav.setAgentEditorSurfaceHandler(null);
  }, [nav.agentEditing, editor.handleEditorSurfaceChange, nav.setAgentEditorSurfaceHandler]);

  // Register the agent studio edit handler so the sidebar's edit button
  // triggers the full editor flow (loading source, building draft, etc.).
  useEffect(() => {
    if (nav.activeAgentId) {
      nav.setAgentStudioEditHandler(() => {
        void editor.handleOpenEdit(nav.activeAgentId!);
      });
    } else {
      nav.setAgentStudioEditHandler(null);
    }
    return () => nav.setAgentStudioEditHandler(null);
  }, [nav.activeAgentId, editor.handleOpenEdit, nav.setAgentStudioEditHandler]);

  const loadSelectedAgentDetail = useCallback(async (agentId: string) => {
    setDetailLoading(true);

    try {
      const detail = await getAgentDetail(agentId);
      setSelectedAgentDetail(detail);
      setSelectedProviderId(detail?.provider_id ?? 'auto');
      setThinkingEffort((detail?.thinking_effort as ThinkingEffort | null | undefined) ?? null);
      setPlanMode((detail?.plan_mode as PlanMode | null | undefined) ?? 'fast');
      if (detail?.mcp_mode) {
        const mode = detail.mcp_mode as McpMode;
        setMcpModeBySession((prev) => ({ ...prev, [mcpSessionKey]: mode }));
      }
      if (detail?.mcp_servers && detail.mcp_servers.length > 0) {
        setMcpServersBySession((prev) => ({ ...prev, [mcpSessionKey]: detail.mcp_servers }));
      }
      return detail;
    } finally {
      setDetailLoading(false);
    }
  }, [getAgentDetail, mcpSessionKey]);

  useEffect(() => {
    if (!nav.activeAgentId) {
      setSelectedAgentDetail(null);
      setSelectedProviderId('auto');
      setThinkingEffort(null);
      setPlanMode('fast');
      return;
    }

    void loadSelectedAgentDetail(nav.activeAgentId);
  }, [loadSelectedAgentDetail, nav.activeAgentId]);

  useEffect(() => {
    if (!nav.activeAgentId || agentActiveSessionId || agentSessions.length === 0) return;
    selectAgentSession(agentSessions[0].id);
  }, [agentActiveSessionId, agentSessions, nav.activeAgentId, selectAgentSession]);

  useEffect(() => {
    if (agentActiveSessionId) {
      void loadMessages(agentActiveSessionId);
    } else if (nav.activeAgentId) {
      clearMessages();
    }
  }, [agentActiveSessionId, clearMessages, loadMessages, nav.activeAgentId]);

  const diagnostics = useDiagnostics(sessionHooks.activeSessionId);
  const statusBarMeta = useStatusBarMeta({
    activeSessionId: sessionHooks.activeSessionId,
    messages: agentChatHooks.messages,
    isStreaming: agentChatHooks.isStreaming,
    isLoadingMessages: agentChatHooks.isLoadingMessages,
    diagnosticEntries: diagnostics.entries,
    isDiagnosticsActive: diagnostics.isActive,
    rootAgentNames: agentRootNames,
  });
  const interactions = useSessionInteractions(sessionHooks.activeSessionId);

  const handleReloadAgents = useCallback(async () => {
    setReloadingAgents(true);

    try {
      const ok = await reloadAgents();
      if (ok && nav.activeAgentId) {
        await loadSelectedAgentDetail(nav.activeAgentId);
      }
    } finally {
      setReloadingAgents(false);
    }
  }, [loadSelectedAgentDetail, nav.activeAgentId, reloadAgents]);

  const handleForkMessage = useCallback(async (messageIndex: number) => {
    if (!sessionHooks.activeSessionId) return;
    const fork = await sessionHooks.forkSession(sessionHooks.activeSessionId, messageIndex);
    // If the original session belongs to a workspace, assign the fork to the same workspace.
    if (fork) {
      const workspaceId = workspaceHooks.sessionWorkspaceMap[sessionHooks.activeSessionId];
      if (workspaceId) {
        await workspaceHooks.assignSession(workspaceId, fork.id);
      }
    }
  }, [sessionHooks, workspaceHooks]);

  const inputDisabled = detailLoading
    || agentChatHooks.isStreaming
    || agentChatHooks.opStatus === 'compacting'
    || (agentChatHooks.opStatus !== 'idle' && agentChatHooks.opStatus !== 'sending');

  const chatHandlers = useChatHandlers({
    activeSessionId: sessionHooks.activeSessionId,
    createSession: sessionHooks.createSession,
    selectSession: sessionHooks.selectSession,
    deleteSession: sessionHooks.deleteSession,
    refreshSessions: sessionHooks.refreshSessions,
    clearMessages: agentChatHooks.clearMessages,
    sendMessage: agentChatHooks.sendMessage,
    editAndResend: agentChatHooks.editAndResend,
    editMessage: agentChatHooks.editMessage,
    cancelEdit: agentChatHooks.cancelEdit,
    undoToMessage: agentChatHooks.undoToMessage,
    resendLastTurn: agentChatHooks.resendLastTurn,
    restoreBranch: agentChatHooks.restoreBranch,
    pendingEdit: agentChatHooks.pendingEdit,
    loadMessages: agentChatHooks.loadMessages,
    selectedProviderId,
    thinkingEffort,
    planMode,
    welcomeWorkspaceId: null,
    assignSession: async () => {},
    refreshWorkspaces: async () => {},
    addUserMessage: diagnostics.addUserMessage,
    addCompactPoint: agentChatHooks.addCompactPoint,
    setOp: agentChatHooks.setOp,
    setActiveView: nav.setActiveView,
    setDiagOpen: (fn) => nav.setDiagOpen(fn(nav.diagOpen)),
    setObsOpen: (fn) => nav.setObsOpen(fn(nav.obsOpen)),
    messages: agentChatHooks.messages,
    onSetRewindDraft: setRewindDraft,
  });

  const selectedAgentSummary = useMemo(() => {
    if (!nav.activeAgentId) {
      return null;
    }

    return selectedAgentDetail ?? agents.find((agent) => agent.id === nav.activeAgentId) ?? null;
  }, [agents, nav.activeAgentId, selectedAgentDetail]);

  const filteredAgents = useMemo(() => {
    const query = agentQuery.trim().toLowerCase();

    if (!query) {
      return agents;
    }

    return agents.filter((agent) => (
      [
        agent.id,
        agent.name,
        agent.description,
        agent.mode,
        agent.provider_id ?? '',
      ]
        .join(' ')
        .toLowerCase()
        .includes(query)
    ));
  }, [agentQuery, agents]);

  const availableSkills = useMemo(
    () => skillHooks.skills.map((skill) => skill.name),
    [skillHooks.skills],
  );
  const knowledgeCollectionNames = useMemo(
    () => knowledgeHooks.collections.map((collection) => collection.name),
    [knowledgeHooks.collections],
  );
  const visibleSkills = useMemo(
    () => ((selectedAgentSummary?.features.skills ?? false)
      ? skillHooks.skills.filter((skill) => skill.enabled)
      : []),
    [selectedAgentSummary?.features.skills, skillHooks.skills],
  );
  const visibleKnowledge = selectedAgentSummary?.features.knowledge ? knowledgeHooks.collections : [];

  return (
    <div className="agents-view">
      {nav.agentEditing ? (
        <AgentEditorPanel
          mode={editor.editorMode}
          draft={editor.editorDraft}
          tab={nav.agentEditorTab}
          surface={nav.agentEditorSurface}
          rawToml={editor.editorRawToml}
          rawPath={editor.editorRawPath}
          rawUsesSourceFile={editor.editorRawUsesSourceFile}
          rawError={editor.editorRawError}
          saving={editor.editorSaving}
          canReset={editor.editorMode === 'edit' && !!selectedAgentDetail?.is_overridden}
          agents={agents}
          tools={tools}
          promptSections={promptSections}
          availableSkills={availableSkills}
          knowledgeCollections={knowledgeCollectionNames}
          mcpServers={mcpServerList}
          providerOptions={providerHooks.providers}
          onChange={editor.handleEditorDraftChange}
          onRawTomlChange={editor.setEditorRawToml}
          onApplyTemplate={editor.handleApplyTemplate}
          onSave={async () => {
            const ok = await editor.handleSaveEditor();
            if (ok && nav.activeAgentId) {
              await loadSelectedAgentDetail(nav.activeAgentId);
            }
          }}
          onReset={async () => {
            const ok = await editor.handleResetEditor();
            if (ok && nav.activeAgentId) {
              await loadSelectedAgentDetail(nav.activeAgentId);
            }
          }}
        />
      ) : (
        <div className="agents-shell">
          {nav.activeAgentId ? (
              <section className="agents-main-panel">
                <AgentStudio
                  agentSummary={selectedAgentSummary}
                  agentId={nav.activeAgentId}
                  detailLoading={detailLoading}
                  sessions={sessionHooks.sessions}
                  activeSessionId={sessionHooks.activeSessionId}
                  streamingSessionIds={agentChatHooks.streamingSessionIds}
                  messages={agentChatHooks.messages}
                  isStreaming={agentChatHooks.isStreaming}
                  isLoadingMessages={agentChatHooks.isLoadingMessages}
                  error={agentChatHooks.error}
                  toolResults={agentChatHooks.toolResults}
                  getStreamSegments={agentChatHooks.getStreamSegments}
                  contextResetPoints={agentChatHooks.contextResetPoints}
                  compactPoints={agentChatHooks.compactPoints}
                  providerCount={providerHooks.systemStatus?.provider_count ?? 0}
                  version={providerHooks.systemStatus?.version ?? 'debug'}
                  activeModel={statusBarMeta.provider}
                  activeProviderIcon={
                    (statusBarMeta.providerId ? providerHooks.providerIconMap[statusBarMeta.providerId] : undefined)
                    ?? (selectedProviderId !== 'auto' ? providerHooks.providerIconMap[selectedProviderId] : undefined)
                    ?? null
                  }
                  lastTokens={statusBarMeta.tokens}
                  lastCost={statusBarMeta.cost}
                  contextWindow={statusBarMeta.contextWindow}
                  contextTokensUsed={statusBarMeta.contextTokensUsed}
                  selectedProviderId={selectedProviderId}
                  thinkingEffort={thinkingEffort}
                  planMode={planMode}
                  inputDisabled={inputDisabled}
                  sendOnEnter={configHooks.config.send_on_enter}
                  providers={providerHooks.providers}
                  providerIcons={providerHooks.providerIconMap}
                  visibleSkills={visibleSkills}
                  visibleKnowledge={visibleKnowledge}
                  inputExpanded={nav.inputExpanded}
                  pendingEdit={agentChatHooks.pendingEdit}
                  isCompacting={agentChatHooks.opStatus === 'compacting'}
                  hasCustomPrompt={sessionHooks.sessions.find((session) => session.id === sessionHooks.activeSessionId)?.has_custom_prompt ?? false}
                  rewindDraft={rewindDraft}
                  mcpMode={mcpMode}
                  onMcpModeChange={handleMcpModeChange}
                  mcpServerList={mcpServerList}
                  selectedMcpServers={selectedMcpServers}
                  onMcpServerToggle={handleMcpServerToggle}
                  askUserData={interactions.askUserData}
                  permissionData={interactions.permissionData}
                  onNewSession={() => void chatHandlers.handleNewChat()}
                  onForkMessage={(messageIndex) => void handleForkMessage(messageIndex)}
                  onSend={chatHandlers.handleSend}
                  onStop={agentChatHooks.cancelRun}
                  onCommand={chatHandlers.handleCommand}
                  onSelectProvider={setSelectedProviderId}
                  onThinkingEffortChange={setThinkingEffort}
                  onPlanModeChange={setPlanMode}
                  onExpandChange={nav.setInputExpanded}
                  onCancelEdit={chatHandlers.handleCancelEdit}
                  onClearSession={() => void chatHandlers.handleClearSession()}
                  onAddContextReset={agentChatHooks.addContextReset}
                  onEditMessage={chatHandlers.handleEditMessage}
                  onUndoMessage={chatHandlers.handleUndoMessage}
                  onResendMessage={chatHandlers.handleResendMessage}
                  onRestoreBranch={chatHandlers.handleRestoreBranch}
                  onCustomPromptChange={() => { void refreshAgentSessions(); }}
                  onRewindDraftConsumed={() => setRewindDraft(null)}
                  onAskUserSubmit={interactions.handleAskUserSubmit}
                  onAskUserDismiss={interactions.handleAskUserDismiss}
                  onPermissionApprove={interactions.handlePermissionApprove}
                  onPermissionDeny={interactions.handlePermissionDeny}
                  onPermissionAllowAllForSession={interactions.handlePermissionAllowAllForSession}
                />
              </section>
          ) : (
              <AgentOverview
                filteredAgents={filteredAgents}
                totalCount={agents.length}
                agentQuery={agentQuery}
                reloading={reloadingAgents}
                onQueryChange={setAgentQuery}
                onSelectAgent={nav.setActiveAgentId}
                onOpenEdit={(id) => void editor.handleOpenEdit(id)}
                onReload={() => void handleReloadAgents()}
                onNewAgent={editor.handleOpenCreate}
              />
          )}
        </div>
      )}
    </div>
  );
}
