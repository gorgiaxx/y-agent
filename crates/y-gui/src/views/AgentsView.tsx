import { useCallback, useEffect, useMemo, useState } from 'react';
import { AgentsSidebarPanel } from '../components/agents/AgentsSidebarPanel';
import { AgentEditorDialog } from '../components/agents/AgentEditorDialog';
import { AgentOverview } from '../components/agents/AgentOverview';
import { AgentStudio } from '../components/agents/AgentStudio';
import {
  useChatContext,
  useNavigationContext,
  useAgentsContext,
  useProvidersContext,
  useConfigContext,
  useSkillsContext,
  useKnowledgeContext,
  useWorkspacesContext,
} from '../providers/AppContexts';
import { useSessions } from '../hooks/useSessions';
import { useChatHandlers } from '../hooks/useChatHandlers';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useSessionInteractions } from '../hooks/useSessionInteractions';
import { useStatusBarMeta } from '../hooks/useStatusBarMeta';
import type { AgentDetail } from '../hooks/useAgents';
import type { PlanMode, ThinkingEffort } from '../types';
import type { EditorTab, EditorSurface, AgentDraft } from '../components/agents/types';
import { buildDraft, serializeAgentDraft, slugifyAgentId } from '../components/agents/utils';
import './AgentsView.css';

export function AgentsView() {
  const nav = useNavigationContext();
  const chatHooks = useChatContext();
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
  const sessionHooks = useSessions(nav.activeAgentId);
  const {
    activeSessionId: agentActiveSessionId,
    sessions: agentSessions,
    selectSession: selectAgentSession,
    refreshSessions: refreshAgentSessions,
  } = sessionHooks;
  const { loadMessages, clearMessages } = chatHooks;

  const [selectedAgentDetail, setSelectedAgentDetail] = useState<AgentDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort | null>(null);
  const [planMode, setPlanMode] = useState<PlanMode>('fast');
  const [rewindDraft, setRewindDraft] = useState<string | null>(null);

  const [editorOpen, setEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<'create' | 'edit'>('create');
  const [editorTab, setEditorTab] = useState<EditorTab>('general');
  const [editorSurface, setEditorSurface] = useState<EditorSurface>('form');
  const [editorDraft, setEditorDraft] = useState<AgentDraft>(buildDraft());
  const [editorRawToml, setEditorRawToml] = useState('');
  const [editorRawPath, setEditorRawPath] = useState<string | null>(null);
  const [editorRawUsesSourceFile, setEditorRawUsesSourceFile] = useState(false);
  const [editorRawOrigin, setEditorRawOrigin] = useState<'form' | 'raw' | 'source'>('form');
  const [editorRawError, setEditorRawError] = useState<string | null>(null);
  const [editorSaving, setEditorSaving] = useState(false);
  const [agentQuery, setAgentQuery] = useState('');
  const [reloadingAgents, setReloadingAgents] = useState(false);

  const loadSelectedAgentDetail = useCallback(async (agentId: string) => {
    setDetailLoading(true);

    try {
      const detail = await getAgentDetail(agentId);
      setSelectedAgentDetail(detail);
      setSelectedProviderId(detail?.provider_id ?? 'auto');
      setThinkingEffort((detail?.thinking_effort as ThinkingEffort | null | undefined) ?? null);
      setPlanMode((detail?.plan_mode as PlanMode | null | undefined) ?? 'fast');
      return detail;
    } finally {
      setDetailLoading(false);
    }
  }, [getAgentDetail]);

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
    messages: chatHooks.messages,
    isStreaming: chatHooks.isStreaming,
    isLoadingMessages: chatHooks.isLoadingMessages,
    diagnosticEntries: diagnostics.entries,
    isDiagnosticsActive: diagnostics.isActive,
    rootAgentNames: nav.activeAgentId ? [nav.activeAgentId] : ['chat-turn'],
  });
  const interactions = useSessionInteractions(sessionHooks.activeSessionId);

  const handleOpenCreate = useCallback(() => {
    const draft = buildDraft();
    setEditorMode('create');
    setEditorTab('general');
    setEditorSurface('form');
    setEditorDraft(draft);
    setEditorRawToml(serializeAgentDraft(draft));
    setEditorRawPath(null);
    setEditorRawUsesSourceFile(false);
    setEditorRawOrigin('form');
    setEditorRawError(null);
    setEditorOpen(true);
  }, []);

  const handleOpenEdit = useCallback(async (agentId: string) => {
    const [detail, source] = await Promise.all([
      getAgentDetail(agentId),
      getAgentSource(agentId),
    ]);
    if (!detail) return;
    setEditorMode('edit');
    setEditorTab('general');
    setEditorSurface('form');
    setEditorDraft(buildDraft(detail));
    setEditorRawToml(source?.content ?? serializeAgentDraft(buildDraft(detail)));
    setEditorRawPath(source?.path ?? null);
    setEditorRawUsesSourceFile(source?.is_user_file ?? false);
    setEditorRawOrigin(source ? 'source' : 'form');
    setEditorRawError(null);
    setEditorOpen(true);
  }, [getAgentDetail, getAgentSource]);

  const handleApplyTemplate = useCallback(async (agentId: string) => {
    const detail = await getAgentDetail(agentId);
    if (!detail) return;
    const nextDraft = {
      ...buildDraft(detail),
      id: '',
      name: `Copy ${detail.name}`,
    };
    setEditorDraft(nextDraft);
    setEditorSurface('form');
    setEditorRawToml(serializeAgentDraft(nextDraft));
    setEditorRawPath(null);
    setEditorRawUsesSourceFile(false);
    setEditorRawOrigin('form');
    setEditorRawError(null);
  }, [getAgentDetail]);

  const handleEditorDraftChange = useCallback((updater: (draft: AgentDraft) => AgentDraft) => {
    setEditorRawOrigin('form');
    setEditorRawError(null);
    setEditorDraft((prev) => updater(prev));
  }, []);

  useEffect(() => {
    if (!editorOpen || editorSurface !== 'form' || editorRawOrigin !== 'form') {
      return;
    }
    setEditorRawToml(serializeAgentDraft(editorDraft));
  }, [editorDraft, editorOpen, editorRawOrigin, editorSurface]);

  const handleEditorSurfaceChange = useCallback(async (surface: EditorSurface) => {
    if (surface === editorSurface) {
      return;
    }

    setEditorRawError(null);

    if (surface === 'raw') {
      setEditorSurface('raw');
      return;
    }

    const parsed = await parseAgentToml(editorRawToml);
    if (!parsed) {
      setEditorRawError('Raw TOML has syntax or schema errors. Fix it before returning to the form editor.');
      return;
    }

    setEditorDraft(buildDraft(parsed));
    setEditorRawOrigin('form');
    setEditorSurface('form');
  }, [editorRawToml, editorSurface, parseAgentToml]);

  const handleSaveEditor = useCallback(async () => {
    let nextId = editorMode === 'edit' ? editorDraft.id : (editorDraft.id.trim() || slugifyAgentId(editorDraft.name));
    let nextContent = serializeAgentDraft({
      ...editorDraft,
      id: nextId,
    });

    if (editorSurface === 'raw') {
      const parsed = await parseAgentToml(editorRawToml);
      if (!parsed) {
        setEditorRawError('Raw TOML has syntax or schema errors. Fix it before saving.');
        return;
      }

      if (editorMode === 'edit') {
        if (parsed.id.trim() && parsed.id.trim() !== editorDraft.id) {
          setEditorRawError('Existing agent IDs cannot be changed in raw mode.');
          return;
        }
        nextId = editorDraft.id;
      } else {
        nextId = parsed.id.trim();
      }

      if (!nextId || !parsed.name.trim()) {
        setEditorRawError('Raw TOML must include both non-empty id and name fields before saving.');
        return;
      }

      nextContent = editorRawToml;
    } else if (!nextId || !editorDraft.name.trim()) {
      return;
    }

    setEditorSaving(true);
    const ok = await saveAgent(nextId, nextContent);
    setEditorSaving(false);
    if (!ok) return;

    setEditorOpen(false);

    if (nav.activeAgentId === nextId) {
      await loadSelectedAgentDetail(nextId);
    }
  }, [editorDraft, editorMode, editorRawToml, editorSurface, loadSelectedAgentDetail, nav.activeAgentId, parseAgentToml, saveAgent]);

  const handleResetEditor = useCallback(async () => {
    if (editorMode !== 'edit') return;

    const ok = await resetAgent(editorDraft.id);
    if (!ok) return;

    setEditorOpen(false);

    if (nav.activeAgentId === editorDraft.id) {
      await loadSelectedAgentDetail(editorDraft.id);
    }
  }, [editorDraft.id, editorMode, loadSelectedAgentDetail, nav.activeAgentId, resetAgent]);

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
    || chatHooks.isStreaming
    || chatHooks.opStatus === 'compacting'
    || (chatHooks.opStatus !== 'idle' && chatHooks.opStatus !== 'sending');

  const chatHandlers = useChatHandlers({
    activeSessionId: sessionHooks.activeSessionId,
    createSession: sessionHooks.createSession,
    selectSession: sessionHooks.selectSession,
    deleteSession: sessionHooks.deleteSession,
    refreshSessions: sessionHooks.refreshSessions,
    clearMessages: chatHooks.clearMessages,
    sendMessage: chatHooks.sendMessage,
    editAndResend: chatHooks.editAndResend,
    editMessage: chatHooks.editMessage,
    cancelEdit: chatHooks.cancelEdit,
    undoToMessage: chatHooks.undoToMessage,
    resendLastTurn: chatHooks.resendLastTurn,
    restoreBranch: chatHooks.restoreBranch,
    pendingEdit: chatHooks.pendingEdit,
    loadMessages: chatHooks.loadMessages,
    selectedProviderId,
    welcomeWorkspaceId: null,
    assignSession: async () => {},
    refreshWorkspaces: async () => {},
    addUserMessage: diagnostics.addUserMessage,
    addCompactPoint: chatHooks.addCompactPoint,
    setOp: chatHooks.setOp,
    setActiveView: nav.setActiveView,
    setDiagOpen: (fn) => nav.setDiagOpen(fn(nav.diagOpen)),
    setObsOpen: (fn) => nav.setObsOpen(fn(nav.obsOpen)),
    messages: chatHooks.messages,
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

  const editorDialog = editorOpen ? (
    <AgentEditorDialog
      mode={editorMode}
      draft={editorDraft}
      tab={editorTab}
      surface={editorSurface}
      rawToml={editorRawToml}
      rawPath={editorRawPath}
      rawUsesSourceFile={editorRawUsesSourceFile}
      rawError={editorRawError}
      saving={editorSaving}
      canReset={editorMode === 'edit' && !!selectedAgentDetail?.is_overridden}
      agents={agents}
      tools={tools}
      promptSections={promptSections}
      availableSkills={availableSkills}
      knowledgeCollections={knowledgeCollectionNames}
      providerOptions={providerHooks.providers}
      onChange={handleEditorDraftChange}
      onTabChange={setEditorTab}
      onSurfaceChange={(surface) => { void handleEditorSurfaceChange(surface); }}
      onRawTomlChange={(content) => {
        setEditorRawToml(content);
        setEditorRawOrigin('raw');
        setEditorRawError(null);
      }}
      onApplyTemplate={handleApplyTemplate}
      onClose={() => setEditorOpen(false)}
      onSave={() => void handleSaveEditor()}
      onReset={() => void handleResetEditor()}
    />
  ) : null;

  return (
    <div className="agents-view">
      <div className="agents-shell">
        {nav.activeAgentId ? (
          <>
            <AgentsSidebarPanel
              agents={filteredAgents}
              activeAgentId={nav.activeAgentId}
              query={agentQuery}
              totalCount={agents.length}
              reloading={reloadingAgents}
              onQueryChange={setAgentQuery}
              onSelectAgent={nav.setActiveAgentId}
              onReload={() => void handleReloadAgents()}
              onNewAgent={handleOpenCreate}
            />

            <section className="agents-main-panel">
              <AgentStudio
                agentSummary={selectedAgentSummary}
                agentId={nav.activeAgentId}
                detailLoading={detailLoading}
                sessions={sessionHooks.sessions}
                activeSessionId={sessionHooks.activeSessionId}
                messages={chatHooks.messages}
                isStreaming={chatHooks.isStreaming}
                isLoadingMessages={chatHooks.isLoadingMessages}
                error={chatHooks.error}
                toolResults={chatHooks.toolResults}
                getStreamSegments={chatHooks.getStreamSegments}
                contextResetPoints={chatHooks.contextResetPoints}
                compactPoints={chatHooks.compactPoints}
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
                pendingEdit={chatHooks.pendingEdit}
                isCompacting={chatHooks.opStatus === 'compacting'}
                hasCustomPrompt={sessionHooks.sessions.find((session) => session.id === sessionHooks.activeSessionId)?.has_custom_prompt ?? false}
                rewindDraft={rewindDraft}
                askUserData={interactions.askUserData}
                permissionData={interactions.permissionData}
                onEdit={() => void handleOpenEdit(nav.activeAgentId!)}
                onNewSession={() => void chatHandlers.handleNewChat()}
                onSelectSession={sessionHooks.selectSession}
                onDeleteSession={(id) => void sessionHooks.deleteSession(id)}
                onForkMessage={(messageIndex) => void handleForkMessage(messageIndex)}
                onSend={chatHandlers.handleSend}
                onStop={chatHooks.cancelRun}
                onCommand={chatHandlers.handleCommand}
                onSelectProvider={setSelectedProviderId}
                onThinkingEffortChange={setThinkingEffort}
                onPlanModeChange={setPlanMode}
                onExpandChange={nav.setInputExpanded}
                onCancelEdit={chatHandlers.handleCancelEdit}
                onClearSession={() => void chatHandlers.handleClearSession()}
                onAddContextReset={chatHooks.addContextReset}
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
          </>
        ) : (
            <AgentOverview
              filteredAgents={filteredAgents}
              agentQuery={agentQuery}
              onSelectAgent={nav.setActiveAgentId}
              onOpenEdit={(id) => void handleOpenEdit(id)}
            />
        )}
      </div>

      {editorDialog}
    </div>
  );
}
