import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChatPanel } from '../components/chat-panel/ChatPanel';
import { WelcomePage } from '../components/WelcomePage';
import { InputArea } from '../components/chat-panel/input-area/InputArea';
import { StatusBar } from '../components/chat-panel/StatusBar';
import { WorkspaceDialog } from '../components/chat-panel/WorkspaceDialog';

import { useChatContext, useSessionsContext, useWorkspacesContext, useSkillsContext, useKnowledgeContext, useProvidersContext, useConfigContext, useNavigationContext } from '../providers/AppContexts';
import { useChatHandlers } from '../hooks/useChatHandlers';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useStatusBarMeta } from '../hooks/useStatusBarMeta';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';
import type { ThinkingEffort } from '../types';

export function ChatView() {
  const chatHooks = useChatContext();
  const sessionHooks = useSessionsContext();
  const workspaceHooks = useWorkspacesContext();
  const skillHooks = useSkillsContext();
  const knowledgeHooks = useKnowledgeContext();
  const providerHooks = useProvidersContext();
  const configHooks = useConfigContext();
  const navProps = useNavigationContext();

  // AskUser interaction state.
  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort | null>(null);
  const [askUserData, setAskUserData] = useState<{
    interactionId: string;
    questions: Array<{
      question: string;
      options: string[];
      multi_select?: boolean;
    }>;
  } | null>(null);

  // PermissionRequest interaction state.
  const [permissionData, setPermissionData] = useState<{
    requestId: string;
    toolName: string;
    actionDescription: string;
    reason: string;
    contentPreview?: string | null;
  } | null>(null);

  // Listen for AskUser events from the backend.
  useEffect(() => {
    const unlisten = listen<{
      run_id: string;
      interaction_id: string;
      questions: unknown;
    }>('chat:AskUser', (event) => {
      const { interaction_id, questions } = event.payload;
      setAskUserData({
        interactionId: interaction_id,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        questions: questions as any,
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const handleAskUserSubmit = useCallback((
    interactionId: string,
    answers: Record<string, string>,
  ) => {
    setAskUserData(null);
    invoke('chat_answer_question', {
      interactionId,
      answers: { answers },
    }).catch(console.error);
  }, []);

  const handleAskUserDismiss = useCallback((interactionId: string) => {
    setAskUserData(null);
    invoke('chat_answer_question', {
      interactionId,
      answers: { answers: {} },
    }).catch(console.error);
  }, []);

  // Listen for PermissionRequest events from the backend.
  useEffect(() => {
    const unlisten = listen<{
      run_id: string;
      request_id: string;
      tool_name: string;
      action_description: string;
      reason: string;
      content_preview: string | null;
    }>('chat:PermissionRequest', (event) => {
      const { request_id, tool_name, action_description, reason, content_preview } = event.payload;
      setPermissionData({
        requestId: request_id,
        toolName: tool_name,
        actionDescription: action_description,
        reason,
        contentPreview: content_preview,
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const handlePermissionApprove = useCallback((requestId: string) => {
    setPermissionData(null);
    invoke('chat_answer_permission', {
      requestId,
      decision: 'approve',
    }).catch(console.error);
  }, []);

  const handlePermissionDeny = useCallback((requestId: string) => {
    setPermissionData(null);
    invoke('chat_answer_permission', {
      requestId,
      decision: 'deny',
    }).catch(console.error);
  }, []);

  const handlePermissionAllowAllForSession = useCallback((requestId: string) => {
    setPermissionData(null);
    invoke('chat_answer_permission', {
      requestId,
      decision: 'allow_all_for_session',
    }).catch(console.error);
  }, []);

  const diagnosticsScope = resolveDiagnosticsScope(navProps.activeView, sessionHooks.activeSessionId);
  const { entries, isActive, addUserMessage } = useDiagnostics(diagnosticsScope.sessionId);

  const {
    handleSend,
    handleEditMessage,
    handleUndoMessage,
    handleCancelEdit,
    handleRestoreBranch,
    handleResendMessage,
    handleClearSession,
    handleCreateWorkspace,
    handleCommand,
  } = useChatHandlers({
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
    selectedProviderId: providerHooks.selectedProviderId,
    welcomeWorkspaceId: navProps.welcomeWorkspaceId,
    assignSession: workspaceHooks.assignSession,
    refreshWorkspaces: workspaceHooks.refreshWorkspaces,
    addUserMessage,
    addCompactPoint: chatHooks.addCompactPoint,
    setOp: chatHooks.setOp,
    setActiveView: navProps.setActiveView,
    setDiagOpen: (fn: (prev: boolean) => boolean) => navProps.setDiagOpen(fn(navProps.diagOpen)),
    setObsOpen: (fn: (prev: boolean) => boolean) => navProps.setObsOpen(fn(navProps.obsOpen)),
  });

  const [wsDialogOpen, setWsDialogOpen] = useState(false);

  const inputDisabled = chatHooks.isStreaming
    || chatHooks.opStatus === 'compacting'
    || (chatHooks.opStatus !== 'idle' && chatHooks.opStatus !== 'sending');

  const statusBarMeta = useStatusBarMeta({
    activeSessionId: sessionHooks.activeSessionId,
    messages: chatHooks.messages,
    isStreaming: chatHooks.isStreaming,
    isLoadingMessages: chatHooks.isLoadingMessages,
    diagnosticEntries: entries,
    isDiagnosticsActive: isActive,
  });

  const handleForkMessage = useCallback((messageIndex: number) => {
    if (!sessionHooks.activeSessionId) return;
    sessionHooks.forkSession(sessionHooks.activeSessionId, messageIndex);
  }, [sessionHooks]);

  return (
    <>
      {!navProps.inputExpanded && sessionHooks.activeSessionId && (
        <ChatPanel 
          messages={chatHooks.messages} 
          isStreaming={chatHooks.isStreaming} 
          isLoading={chatHooks.isLoadingMessages} 
          error={chatHooks.error} 
          onEditMessage={handleEditMessage} 
          onUndoMessage={handleUndoMessage} 
          onResendMessage={handleResendMessage} 
          onForkMessage={handleForkMessage}
          onRestoreBranch={handleRestoreBranch} 
          toolResults={chatHooks.toolResults} 
          contextResetPoints={chatHooks.contextResetPoints} 
          compactPoints={chatHooks.compactPoints}
        />
      )}
      {!navProps.inputExpanded && !sessionHooks.activeSessionId && (
        <WelcomePage
          workspaces={workspaceHooks.workspaces}
          selectedWorkspaceId={navProps.welcomeWorkspaceId}
          onSelectWorkspace={navProps.setWelcomeWorkspaceId}
          onCreateWorkspace={() => setWsDialogOpen(true)}
        />
      )}
      <InputArea
        key={sessionHooks.activeSessionId ?? '__no_session__'}
        onSend={handleSend}
        onStop={chatHooks.cancelRun}
        onCommand={handleCommand}
        disabled={inputDisabled}
        sendOnEnter={configHooks.config.send_on_enter}
        providers={providerHooks.providers}
        selectedProviderId={providerHooks.selectedProviderId}
        onSelectProvider={providerHooks.setSelectedProviderId}
        pendingEdit={chatHooks.pendingEdit}
        onCancelEdit={handleCancelEdit}
        skills={skillHooks.skills.filter((s) => s.enabled)}
        knowledgeCollections={knowledgeHooks.collections}
        expanded={navProps.inputExpanded}
        onExpandChange={navProps.setInputExpanded}
        onClearSession={handleClearSession}
        onAddContextReset={chatHooks.addContextReset}
        providerIcons={providerHooks.providerIconMap}
        thinkingEffort={thinkingEffort}
        onThinkingEffortChange={setThinkingEffort}
        askUserData={askUserData}
        onAskUserSubmit={handleAskUserSubmit}
        onAskUserDismiss={handleAskUserDismiss}
        permissionData={permissionData}
        onPermissionApprove={handlePermissionApprove}
        onPermissionDeny={handlePermissionDeny}
        onPermissionAllowAllForSession={handlePermissionAllowAllForSession}
        isCompacting={chatHooks.opStatus === 'compacting'}
      />
      <StatusBar
        providerCount={providerHooks.systemStatus?.provider_count ?? 0}
        sessionCount={providerHooks.systemStatus?.session_count ?? null}
        version={providerHooks.systemStatus?.version ?? 'debug'}
        activeModel={statusBarMeta.provider}
        activeProviderIcon={
          (statusBarMeta.providerId ? providerHooks.providerIconMap[statusBarMeta.providerId] : undefined)
          ?? (providerHooks.selectedProviderId !== 'auto' ? providerHooks.providerIconMap[providerHooks.selectedProviderId] : undefined)
          ?? null
        }
        lastTokens={statusBarMeta.tokens}
        lastCost={statusBarMeta.cost}
        contextWindow={statusBarMeta.contextWindow}
        contextTokensUsed={statusBarMeta.contextTokensUsed}
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
    </>
  );
}
