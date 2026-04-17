import { useState, useCallback } from 'react';
import { ChatPanel } from '../components/chat-panel/ChatPanel';
import { WelcomePage } from '../components/WelcomePage';
import { InputArea } from '../components/chat-panel/input-area/InputArea';
import { StatusBar } from '../components/chat-panel/StatusBar';
import { WorkspaceDialog } from '../components/chat-panel/WorkspaceDialog';
import { RewindPanel } from '../components/chat-panel/RewindPanel';
import { useRewind } from '../hooks/useRewind';
import { useMcpServers } from '../hooks/useMcpServers';

import { useChatContext, useSessionsContext, useWorkspacesContext, useSkillsContext, useKnowledgeContext, useProvidersContext, useConfigContext, useNavigationContext } from '../providers/AppContexts';
import { useChatHandlers } from '../hooks/useChatHandlers';
import { useDiagnostics } from '../hooks/useDiagnostics';
import { useSessionInteractions } from '../hooks/useSessionInteractions';
import { useStatusBarMeta } from '../hooks/useStatusBarMeta';
import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';
import type { ThinkingEffort, PlanMode, McpMode } from '../types';

export function ChatView() {
  const chatHooks = useChatContext();
  const sessionHooks = useSessionsContext();
  const workspaceHooks = useWorkspacesContext();
  const skillHooks = useSkillsContext();
  const knowledgeHooks = useKnowledgeContext();
  const providerHooks = useProvidersContext();
  const configHooks = useConfigContext();
  const navProps = useNavigationContext();

  const rewind = useRewind();

  // Draft text to populate in the input box after rewind/undo.
  const [rewindDraft, setRewindDraft] = useState<string | null>(null);

  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort | null>(null);

  // MCP mode + manual-mode server selection are per-session, remembered per active session.
  const [mcpModeBySession, setMcpModeBySession] = useState<Record<string, McpMode>>({});
  const [mcpServersBySession, setMcpServersBySession] = useState<Record<string, string[]>>({});
  const mcpSessionKey = sessionHooks.activeSessionId ?? '__no_session__';
  const mcpMode: McpMode = mcpModeBySession[mcpSessionKey] ?? 'auto';
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
  const {
    askUserData,
    permissionData,
    handleAskUserSubmit,
    handleAskUserDismiss,
    handlePermissionApprove,
    handlePermissionDeny,
    handlePermissionAllowAllForSession,
  } = useSessionInteractions(sessionHooks.activeSessionId);

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
    messages: chatHooks.messages,
    selectedProviderId: providerHooks.selectedProviderId,
    thinkingEffort,
    planMode: 'fast' as PlanMode,
    welcomeWorkspaceId: navProps.welcomeWorkspaceId,
    assignSession: workspaceHooks.assignSession,
    refreshWorkspaces: workspaceHooks.refreshWorkspaces,
    addUserMessage,
    addCompactPoint: chatHooks.addCompactPoint,
    setOp: chatHooks.setOp,
    setActiveView: navProps.setActiveView,
    setDiagOpen: (fn: (prev: boolean) => boolean) => navProps.setDiagOpen(fn(navProps.diagOpen)),
    setObsOpen: (fn: (prev: boolean) => boolean) => navProps.setObsOpen(fn(navProps.obsOpen)),
    onRewind: () => {
      if (sessionHooks.activeSessionId) {
        rewind.open(sessionHooks.activeSessionId);
      }
    },
    onSetRewindDraft: setRewindDraft,
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
          getStreamSegments={chatHooks.getStreamSegments}
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
        sessionId={sessionHooks.activeSessionId}
        hasCustomPrompt={
          sessionHooks.sessions.find((s) => s.id === sessionHooks.activeSessionId)
            ?.has_custom_prompt ?? false
        }
        onCustomPromptChange={(hasPrompt) => {
          sessionHooks.refreshSessions?.();
          void hasPrompt;
        }}
        rewindDraft={rewindDraft}
        onRewindDraftConsumed={() => setRewindDraft(null)}
        mcpMode={mcpMode}
        onMcpModeChange={handleMcpModeChange}
        mcpServerList={mcpServerList}
        selectedMcpServers={selectedMcpServers}
        onMcpServerToggle={handleMcpServerToggle}
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
    </>
  );
}
