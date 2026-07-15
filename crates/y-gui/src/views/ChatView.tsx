import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { ChatPanel } from '../components/chat-panel/ChatPanel';
import { ChatSearchProvider } from '../components/chat-panel/ChatSearchContext';
import { WelcomePage } from '../components/WelcomePage';
import { InputArea } from '../components/chat-panel/input-area/InputArea';
import { SteeringQueue } from '../components/chat-panel/SteeringQueue';
import { TodoQueue } from '../components/chat-panel/TodoQueue';
import { StatusBar } from '../components/chat-panel/StatusBar';
import { WorkspaceDialog } from '../components/chat-panel/WorkspaceDialog';
import { RewindPanel } from '../components/chat-panel/RewindPanel';
import { useRewind } from '../hooks/useRewind';
import { useMcpServers } from '../hooks/useMcpServers';
import { useToast } from '../hooks/useToast';

import { useChatContext, useSessionsContext, useWorkspacesContext, useSkillsContext, useKnowledgeContext, useProvidersContext, useConfigContext, useViewRouting, usePanelContext, useBackgroundTasksContext, useSteeringContext, useTodoQueueContext } from '../providers/AppContexts';
import { useChatHandlers } from '../hooks/useChatHandlers';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useSessionInteractions } from '../hooks/useSessionInteractions';
import { getVisiblePendingEdit } from '../hooks/chatEditState';
import { PlanReviewProvider } from '../components/chat-panel/PlanReviewContext';
import { useStatusBarMeta } from '../hooks/useStatusBarMeta';
import {
  createSessionInputStates,
  getSessionInputState,
  setSessionDraft,
  setSessionProvider,
  type SessionInputDraft,
} from '../hooks/sessionInputState';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';
import type { ThinkingEffort, PlanMode, McpMode, RequestMode, SteerMessage, TodoItem } from '../types';

export function ChatView() {
  const chatHooks = useChatContext();
  const steering = useSteeringContext();
  const todoQueue = useTodoQueueContext();
  const { toast } = useToast();
  const sessionHooks = useSessionsContext();
  const workspaceHooks = useWorkspacesContext();
  const skillHooks = useSkillsContext();
  const knowledgeHooks = useKnowledgeContext();
  const providerHooks = useProvidersContext();
  const configHooks = useConfigContext();
  const viewRouting = useViewRouting();
  const panelCtx = usePanelContext();
  const backgroundTasks = useBackgroundTasksContext();
  const createSession = sessionHooks.createSession;

  const rewind = useRewind();

  // Draft text to populate in the input box after rewind/undo.
  const [rewindDraft, setRewindDraft] = useState<string | null>(null);

  const sessionInputKey = sessionHooks.activeSessionId ?? '__no_session__';
  const [sessionInputStates, setSessionInputStates] = useState(createSessionInputStates);
  const activeInputState = getSessionInputState(
    sessionInputStates,
    sessionInputKey,
    providerHooks.selectedProviderId,
  );
  const handleDraftChange = useCallback((draft: SessionInputDraft) => {
    setSessionInputStates((previous) => setSessionDraft(previous, sessionInputKey, draft));
  }, [sessionInputKey]);
  const handleProviderChange = useCallback((providerId: string) => {
    setSessionInputStates((previous) => (
      setSessionProvider(previous, sessionInputKey, providerId)
    ));
  }, [sessionInputKey]);
  const createSessionWithInputState = useCallback(async (title?: string) => {
    const session = await createSession(title);
    if (session) {
      setSessionInputStates((previous) => (
        setSessionProvider(previous, session.id, activeInputState.providerId)
      ));
    }
    return session;
  }, [activeInputState.providerId, createSession]);

  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort | null>(null);
  const [planMode, setPlanMode] = useState<PlanMode>(() => {
    try {
      const stored = localStorage.getItem('y-agent-plan-mode');
      if (stored === 'fast' || stored === 'auto' || stored === 'plan' || stored === 'loop') {
        return stored;
      }
    } catch { /* ignore */ }
    return 'fast';
  });
  const handlePlanModeChange = useCallback((mode: PlanMode) => {
    setPlanMode(mode);
    try {
      localStorage.setItem('y-agent-plan-mode', mode);
    } catch { /* ignore */ }
  }, []);

  // MCP mode + manual-mode server selection are per-session, remembered per active session.
  const [mcpModeBySession, setMcpModeBySession] = useState<Record<string, McpMode>>({});
  const [mcpServersBySession, setMcpServersBySession] = useState<Record<string, string[]>>({});
  const mcpSessionKey = sessionInputKey;
  const mcpMode: McpMode = mcpModeBySession[mcpSessionKey] ?? 'disabled';
  const selectedMcpServers = mcpServersBySession[mcpSessionKey] ?? [];

  // Request mode (text_chat vs image_generation) is per-session.
  const [requestModeBySession, setRequestModeBySession] = useState<Record<string, RequestMode>>({});
  const sessionRequestMode: RequestMode = requestModeBySession[mcpSessionKey] ?? 'text_chat';
  const handleRequestModeChange = useCallback((mode: RequestMode) => {
    setRequestModeBySession((prev) => ({ ...prev, [mcpSessionKey]: mode }));
  }, [mcpSessionKey]);
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
  const {
    askUserData,
    permissionData,
    pendingReviewIds,
    handleAskUserSubmit,
    handleAskUserDismiss,
    handlePermissionApprove,
    handlePermissionDeny,
    handlePermissionAllowAllForSession,
    handlePermissionApproveAlways,
    handlePlanReviewApprove,
    handlePlanReviewRevise,
    handlePlanReviewReject,
    handlePlanExecutionRevision,
  } = useSessionInteractions(sessionHooks.activeSessionId);

  const diagnosticsScope = resolveDiagnosticsScope(viewRouting.activeView, sessionHooks.activeSessionId);
  const { entries, isActive, addUserMessage } = useDiagnostics(diagnosticsScope.sessionId);
  const visiblePendingEdit = getVisiblePendingEdit(
    chatHooks.pendingEdit,
    sessionHooks.activeSessionId,
  );

  const {
    handleSend,
    handleEditMessage,
    handleUndoMessage,
    handleCancelEdit,
    handleRestoreBranch,
    handleResendMessage,
    handleRetryTurn,
    handleClearSession,
    handleCreateWorkspace,
    handleCommand,
  } = useChatHandlers({
    session: {
      activeSessionId: sessionHooks.activeSessionId,
      createSession: createSessionWithInputState,
      selectSession: sessionHooks.selectSession,
      deleteSession: sessionHooks.deleteSession,
      refreshSessions: sessionHooks.refreshSessions,
    },
    chat: {
      clearMessages: chatHooks.clearMessages,
      purgeSession: chatHooks.purgeSession,
      sendMessage: chatHooks.sendMessage,
      editAndResend: chatHooks.editAndResend,
      editMessage: chatHooks.editMessage,
      cancelEdit: chatHooks.cancelEdit,
      undoToMessage: chatHooks.undoToMessage,
      resendLastTurn: chatHooks.resendLastTurn,
      restoreBranch: chatHooks.restoreBranch,
      pendingEdit: visiblePendingEdit,
      loadMessages: chatHooks.loadMessages,
      messages: chatHooks.messages,
      addCompactPoint: chatHooks.addCompactPoint,
      setOp: chatHooks.setOp,
    },
    workspace: {
      welcomeWorkspaceId: viewRouting.welcomeWorkspaceId,
      assignSession: workspaceHooks.assignSession,
      refreshWorkspaces: workspaceHooks.refreshWorkspaces,
    },
    config: {
      selectedProviderId: activeInputState.providerId,
      thinkingEffort,
      planMode,
    },
    callbacks: {
      addUserMessage,
      setActiveView: viewRouting.setActiveView,
      setDiagOpen: (fn: (prev: boolean) => boolean) => panelCtx.setDiagOpen(fn(panelCtx.diagOpen)),
      setObsOpen: (fn: (prev: boolean) => boolean) => panelCtx.setObsOpen(fn(panelCtx.obsOpen)),
      onRewind: () => {
        if (sessionHooks.activeSessionId) {
          rewind.open(sessionHooks.activeSessionId);
        }
      },
      onSetRewindDraft: setRewindDraft,
    },
  });

  const [wsDialogOpen, setWsDialogOpen] = useState(false);

  const inputDisabled = chatHooks.isStreaming
    || chatHooks.opStatus === 'compacting'
    || (chatHooks.opStatus !== 'idle' && chatHooks.opStatus !== 'sending');

  // Steering: while a run is streaming, typed messages are queued and injected
  // at the next LLM-call boundary instead of starting a new turn.
  const steerActive = chatHooks.isStreaming;
  const activeSteers: SteerMessage[] = steering.steersFor(sessionHooks.activeSessionId);
  const activeTodos: TodoItem[] = todoQueue.todosFor(sessionHooks.activeSessionId);

  const handleSteer = useCallback((text: string) => {
    if (sessionHooks.activeSessionId) {
      void steering.addSteer(sessionHooks.activeSessionId, text);
    }
  }, [steering, sessionHooks.activeSessionId]);

  const handleSteerEdit = useCallback((steer: SteerMessage) => {
    if (sessionHooks.activeSessionId) {
      void steering.deleteSteer(sessionHooks.activeSessionId, steer.id);
      // Reuse the rewind-draft channel to repopulate the input box.
      setRewindDraft(steer.text);
    }
  }, [steering, sessionHooks.activeSessionId]);

  const handleSteerDelete = useCallback((steerId: string) => {
    if (sessionHooks.activeSessionId) {
      void steering.deleteSteer(sessionHooks.activeSessionId, steerId);
    }
  }, [steering, sessionHooks.activeSessionId]);

  const handleTodo = useCallback(async (text: string) => {
    const sid = sessionHooks.activeSessionId;
    if (!text.trim()) {
      toast('Enter a task after /todo', 'error');
      return;
    }
    if (!sid || !chatHooks.isStreaming) {
      toast('TODO can only be added while the assistant is running', 'error');
      return;
    }
    try {
      await todoQueue.addTodo(sid, text);
      toast('TODO added', 'success');
    } catch {
      toast('The active run is no longer accepting TODO items', 'error');
    }
  }, [chatHooks.isStreaming, sessionHooks.activeSessionId, todoQueue, toast]);

  const handleTodoEdit = useCallback((todo: TodoItem) => {
    const sid = sessionHooks.activeSessionId;
    if (!sid) return;
    void todoQueue.deleteTodo(sid, todo.id).catch(() => {
      toast('Failed to edit TODO item', 'error');
    });
    setRewindDraft(todo.text);
  }, [sessionHooks.activeSessionId, todoQueue, toast]);

  const handleTodoDelete = useCallback((todoId: string) => {
    const sid = sessionHooks.activeSessionId;
    if (!sid) return;
    void todoQueue.deleteTodo(sid, todoId).catch(() => {
      toast('Failed to delete TODO item', 'error');
    });
  }, [sessionHooks.activeSessionId, todoQueue, toast]);

  // Residual replay: steers added too late to be injected fire one-by-one as
  // new turns. On a streaming->idle transition, dequeue the oldest residual and
  // send it through the normal send path (decision #2). The transition guard
  // ensures we only act once per completed run.
  const wasStreamingRef = useRef(false);
  useEffect(() => {
    const sid = sessionHooks.activeSessionId;
    if (wasStreamingRef.current && !chatHooks.isStreaming && sid) {
      const residuals = steering.popResiduals(sid);
      if (residuals.length > 0) {
        // Send the oldest now; the rest remain queued and replay on the next
        // completion, preserving FIFO order without overlapping turns.
        const [first, ...rest] = residuals;
        for (const r of rest) {
          void steering.addSteer(sid, r.text);
        }
        void handleSend(first.text);
      }
    }
    wasStreamingRef.current = chatHooks.isStreaming;
  }, [chatHooks.isStreaming, sessionHooks.activeSessionId, steering, handleSend]);

  const statusBarMeta = useStatusBarMeta({
    activeSessionId: sessionHooks.activeSessionId,
    messages: chatHooks.messages,
    isStreaming: chatHooks.isStreaming,
    isLoadingMessages: chatHooks.isLoadingMessages,
    diagnosticEntries: entries,
    isDiagnosticsActive: isActive,
  });
  const backgroundTaskTotal = backgroundTasks.tasks.length;
  const backgroundTaskRunning = backgroundTasks.tasks.filter((task) => task.status === 'running').length;
  const backgroundTaskFailed = backgroundTasks.tasks.filter((task) => task.status === 'failed').length;
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

  const planReviewContextValue = useMemo(() => ({
    pendingReviewIds,
    onApprove: handlePlanReviewApprove,
    onRevise: handlePlanReviewRevise,
    onReject: handlePlanReviewReject,
    onRequestExecutionRevision: handlePlanExecutionRevision,
  }), [pendingReviewIds, handlePlanReviewApprove, handlePlanReviewRevise, handlePlanReviewReject, handlePlanExecutionRevision]);

  // A sub-agent child session (drilled-in from the info panel) is managed by
  // its parent's orchestrator, not by `chat_resend`. Showing a Retry button
  // there would truncate the child transcript and destroy the assistant's
  // accumulated work. Retry is only safe on user-facing sessions (main/branch).
  const isUserSession = sessionHooks.sessions.some(
    (s) => s.id === sessionHooks.activeSessionId,
  );

  return (
    <PlanReviewProvider value={planReviewContextValue}>
      {!viewRouting.inputExpanded && sessionHooks.activeSessionId && (
        <ChatSearchProvider>
          <ChatPanel
            messages={chatHooks.messages}
            isStreaming={chatHooks.isStreaming}
            isLoading={chatHooks.isLoadingMessages}
            error={chatHooks.error}
            onEditMessage={handleEditMessage}
            onUndoMessage={handleUndoMessage}
            onResendMessage={handleResendMessage}
            onRetryTurn={isUserSession ? handleRetryTurn : undefined}
            onForkMessage={handleForkMessage}
            onRestoreBranch={handleRestoreBranch}
            toolResults={chatHooks.toolResults}
            getStreamSegments={chatHooks.getStreamSegments}
            contextResetPoints={chatHooks.contextResetPoints}
            onUndoContextReset={chatHooks.removeContextReset}
            compactPoints={chatHooks.compactPoints}
          />
        </ChatSearchProvider>
      )}
      {!viewRouting.inputExpanded && !sessionHooks.activeSessionId && (
        <WelcomePage
          workspaces={workspaceHooks.workspaces}
          selectedWorkspaceId={viewRouting.welcomeWorkspaceId}
          onSelectWorkspace={viewRouting.setWelcomeWorkspaceId}
          onCreateWorkspace={() => setWsDialogOpen(true)}
        />
      )}
      {steerActive && activeSteers.length > 0 && (
        <SteeringQueue
          steers={activeSteers}
          onEdit={handleSteerEdit}
          onDelete={handleSteerDelete}
        />
      )}
      {steerActive && activeTodos.length > 0 && (
        <TodoQueue
          todos={activeTodos}
          onEdit={handleTodoEdit}
          onDelete={handleTodoDelete}
        />
      )}
      <InputArea
        key={sessionHooks.activeSessionId ?? '__no_session__'}
        onSend={handleSend}
        onStop={chatHooks.cancelRun}
        onCommand={handleCommand}
        disabled={inputDisabled}
        steerActive={steerActive}
        onSteer={handleSteer}
        onTodo={(text) => { void handleTodo(text); }}
        sendOnEnter={configHooks.config.send_on_enter}
        skills={skillHooks.skills.filter((s) => s.enabled)}
        knowledgeCollections={knowledgeHooks.collections}
        expanded={viewRouting.inputExpanded}
        onExpandChange={viewRouting.setInputExpanded}
        onClearSession={handleClearSession}
        onAddContextReset={chatHooks.addContextReset}
        isCompacting={chatHooks.opStatus === 'compacting'}
        sessionId={sessionHooks.activeSessionId}
        hasCustomPrompt={
          sessionHooks.sessions.find((s) => s.id === sessionHooks.activeSessionId)
            ?.has_custom_prompt ?? false
        }
        onEditSessionPrompt={() => {
          if (!sessionHooks.activeSessionId) return;
          viewRouting.setSessionPromptSessionId(sessionHooks.activeSessionId);
          viewRouting.setSessionPromptEditing(true);
          viewRouting.setInputExpanded(false);
        }}
        onManagePrompts={() => {
          viewRouting.setActiveView('settings');
          viewRouting.setActiveSettingsTab('promptTemplates');
          viewRouting.setInputExpanded(false);
        }}
        onSessionPromptApplied={() => {
          sessionHooks.refreshSessions();
        }}
        provider={{
          providers: providerHooks.providers,
          selectedProviderId: activeInputState.providerId,
          onSelectProvider: handleProviderChange,
          providerIcons: providerHooks.providerIconMap,
        }}
        mcp={{
          mcpMode,
          onMcpModeChange: handleMcpModeChange,
          mcpServerList,
          selectedMcpServers,
          onMcpServerToggle: handleMcpServerToggle,
        }}
        dialogs={{
          askUserData,
          onAskUserSubmit: handleAskUserSubmit,
          onAskUserDismiss: handleAskUserDismiss,
          permissionData,
          onPermissionApprove: handlePermissionApprove,
          onPermissionDeny: handlePermissionDeny,
          onPermissionAllowAllForSession: handlePermissionAllowAllForSession,
          onPermissionApproveAlways: handlePermissionApproveAlways,
        }}
        edit={{
          pendingEdit: visiblePendingEdit,
          onCancelEdit: handleCancelEdit,
          rewindDraft,
          onRewindDraftConsumed: () => setRewindDraft(null),
        }}
        draft={{
          content: activeInputState.draft,
          onContentChange: handleDraftChange,
        }}
        features={{
          thinkingEffort,
          onThinkingEffortChange: setThinkingEffort,
          planMode,
          onPlanModeChange: handlePlanModeChange,
          persistPlanMode: false,
          requestMode: sessionRequestMode,
          onRequestModeChange: handleRequestModeChange,
        }}
      />
      <StatusBar
        version={providerHooks.systemStatus?.version ?? 'debug'}
        activeModel={statusBarMeta.provider}
        activeProviderIcon={
          (statusBarMeta.providerId ? providerHooks.providerIconMap[statusBarMeta.providerId] : undefined)
          ?? (activeInputState.providerId !== 'auto' ? providerHooks.providerIconMap[activeInputState.providerId] : undefined)
          ?? null
        }
        lastTokens={statusBarMeta.tokens}
        lastCost={statusBarMeta.cost}
        contextWindow={statusBarMeta.contextWindow}
        contextTokensUsed={statusBarMeta.contextTokensUsed}
        cacheReadTokens={statusBarMeta.cacheReadTokens}
        backgroundTasks={{
          total: backgroundTaskTotal,
          running: backgroundTaskRunning,
          failed: backgroundTaskFailed,
          onClick: () => {
            viewRouting.setBackgroundTasksSessionId(sessionHooks.activeSessionId);
            viewRouting.setBackgroundTasksSidebarOpen(true);
            viewRouting.setInputExpanded(false);
          },
        }}
      />

      {wsDialogOpen && (
        <WorkspaceDialog
          onConfirm={(name, path) => {
            handleCreateWorkspace(name, path);
            setWsDialogOpen(false);
          }}
          onClose={() => setWsDialogOpen(false)}
        />
      )}

      {rewind.isOpen && sessionHooks.activeSessionId && (
        <RewindPanel
          points={rewind.points}
          isLoading={rewind.isLoading}
          isRewinding={rewind.isRewinding}
          error={rewind.error}
          onSelect={async (point) => {
            const result = await rewind.executeRewind(
              sessionHooks.activeSessionId!,
              point.message_id,
            );
            if (result) {
              // Reload messages to reflect the rewound state.
              await chatHooks.loadMessages(sessionHooks.activeSessionId!);
              sessionHooks.refreshSessions();
              // Place the rewound message content back in the input box.
              setRewindDraft(point.message_preview);
            }
          }}
          onClose={rewind.close}
        />
      )}
    </PlanReviewProvider>
  );
}
