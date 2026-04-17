// ---------------------------------------------------------------------------
// useChatHandlers -- chat action handlers extracted from App.tsx.
//
// Thin delegation layer that composes useChat operations with session
// management, workspace assignment, and diagnostics integration.
// Also includes the slash-command handler.
// ---------------------------------------------------------------------------

import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ChatStarted, Message, ThinkingEffort, PlanMode, McpMode, Attachment } from '../types';
import type { ViewType } from '../components/Sidebar';
import type { CompactInfo, ChatOpStatus } from './useChat';

export interface ChatDeps {
  // Session deps
  activeSessionId: string | null;
  createSession: (title?: string) => Promise<{ id: string; title: string | null } | null>;
  selectSession: (id: string) => void;
  deleteSession: (id: string) => Promise<void>;
  refreshSessions: () => void;
  clearMessages: () => void;

  // Chat deps
  sendMessage: (message: string, sessionId: string, providerId?: string, skills?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode, mcpMode?: McpMode | null, mcpServers?: string[]) => Promise<ChatStarted | null>;
  editAndResend: (sessionId: string, newContent: string, providerId?: string, thinkingEffort?: ThinkingEffort | null, planMode?: PlanMode) => Promise<ChatStarted | null>;
  editMessage: (messageId: string, content: string) => void;
  cancelEdit: () => void;
  undoToMessage: (sessionId: string, messageId: string) => Promise<unknown>;
  resendLastTurn: (sessionId: string, messageId: string, content: string, providerId?: string, thinkingEffort?: ThinkingEffort | null, planMode?: PlanMode) => Promise<unknown>;
  restoreBranch: (sessionId: string, checkpointId: string) => Promise<unknown>;
  pendingEdit: { messageId: string; content: string } | null;
  loadMessages: (sessionId: string) => Promise<void>;

  // Provider
  selectedProviderId: string;

  // Thinking / Plan mode
  thinkingEffort: ThinkingEffort | null;
  planMode: PlanMode;

  // Workspace
  welcomeWorkspaceId: string | null;
  assignSession: (workspaceId: string, sessionId: string) => Promise<void>;
  refreshWorkspaces: () => Promise<void>;

  // Diagnostics integration
  addUserMessage: (content: string, sessionId: string) => void;

  // Compact
  addCompactPoint: (info: Omit<CompactInfo, 'atIndex'>) => void;
  setOp: (status: ChatOpStatus) => void;

  // Navigation
  setActiveView: (view: ViewType) => void;
  setDiagOpen: (fn: (prev: boolean) => boolean) => void;
  setObsOpen: (fn: (prev: boolean) => boolean) => void;

  // Rewind
  onRewind?: () => void;
  /** Callback to set the input box draft after rewind/undo. */
  onSetRewindDraft?: (content: string) => void;
  /** Current messages (for finding content on undo). */
  messages: Message[];
}

export interface UseChatHandlersReturn {
  handleSend: (message: string, skillNames?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode, mcpMode?: McpMode | null, mcpServers?: string[]) => Promise<void>;
  handleEditMessage: (content: string, messageId: string) => void;
  handleUndoMessage: (messageId: string) => Promise<void>;
  handleCancelEdit: () => void;
  handleRestoreBranch: (checkpointId: string) => Promise<void>;
  handleResendMessage: (content: string, messageId: string) => Promise<void>;
  handleClearSession: () => Promise<void>;
  handleNewChat: () => Promise<void>;
  handleNewChatInWorkspace: (workspaceId: string) => Promise<void>;
  handleDeleteSession: (id: string) => Promise<void>;
  handleCreateWorkspace: (name: string, path: string) => Promise<void>;
  handleCommand: (commandName: string) => boolean;
}

export function useChatHandlers(deps: ChatDeps): UseChatHandlersReturn {
  const {
    activeSessionId,
    createSession,
    selectSession,
    deleteSession,
    refreshSessions,
    clearMessages,
    sendMessage,
    editAndResend,
    editMessage,
    cancelEdit,
    undoToMessage,
    resendLastTurn,
    restoreBranch,
    pendingEdit,
    loadMessages,
    selectedProviderId,
    thinkingEffort,
    planMode,
    welcomeWorkspaceId,
    assignSession,
    refreshWorkspaces,
    addUserMessage,
    addCompactPoint,
    setOp,
    setActiveView,
    setDiagOpen,
    setObsOpen,
    onRewind,
    onSetRewindDraft,
    messages,
  } = deps;

  const handleSend = useCallback(
    async (message: string, skillNames?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode, mcpMode?: McpMode | null, mcpServers?: string[]) => {
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
        const result = await editAndResend(sid, message, providerArg, thinkingEffort, planMode);
        if (result) {
          if (result.session_id !== activeSessionId) {
            selectSession(result.session_id);
          }
          refreshSessions();
        }
        return;
      }

      // Normal send -- pass skills, knowledge collections, attachments, planMode, and mcp mode to the backend.
      addUserMessage(message, sid);
      const result = await sendMessage(message, sid, providerArg, skillNames, knowledgeCollections, thinkingEffort, attachments, planMode, mcpMode, mcpServers);
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

      // Find the message content before undo so we can populate the input.
      const targetMsg = messages.find((m) => m.id === messageId);
      const messageContent = targetMsg?.content ?? '';

      await undoToMessage(activeSessionId, messageId);

      // Restore files to the state before this message was sent.
      // Best-effort: silently succeeds if no file history exists.
      try {
        await invoke('rewind_restore_files', {
          sessionId: activeSessionId,
          targetMessageId: messageId,
        });
      } catch (e) {
        console.warn('[undo] file restoration failed (non-fatal):', e);
      }

      // Put the undone message content back in the input box.
      if (messageContent && onSetRewindDraft) {
        onSetRewindDraft(messageContent);
      }
    },
    [activeSessionId, undoToMessage, messages, onSetRewindDraft],
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
      await resendLastTurn(activeSessionId, messageId, content, providerArg, thinkingEffort, planMode);
    },
    [activeSessionId, resendLastTurn, selectedProviderId, thinkingEffort, planMode],
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

  // Slash-command handler -- maps command names to existing GUI actions.
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
            const sid = activeSessionId;
            setOp('compacting');
            invoke<{ messages_pruned: number; messages_compacted: number; tokens_saved: number; summary: string }>(
              'context_compact',
              { sessionId: sid },
            )
              .then((result) => {
                console.info(
                  `[compact] done: pruned=${result.messages_pruned}, ` +
                  `compacted=${result.messages_compacted}, tokens_saved=${result.tokens_saved}`,
                );
                // Record a compaction point for the divider + summary bubble.
                addCompactPoint({
                  messagesPruned: result.messages_pruned,
                  messagesCompacted: result.messages_compacted,
                  tokensSaved: result.tokens_saved,
                  summary: result.summary,
                });
                // Reload messages so the UI reflects the compacted state.
                loadMessages(sid);
              })
              .catch((e) => console.error('[compact] failed:', e))
              .finally(() => setOp('idle'));
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
          invoke<unknown>('system_status')
            .then(() => console.log('Status refreshed'))
            .catch((e) => console.warn('Failed to refresh system status:', e));
          return true;
        case 'help':
          setActiveView('settings');
          return true;
        case 'export':
          console.log('Export command triggered -- not yet implemented');
          return true;
        case 'model':
          setActiveView('settings');
          return true;
        case 'rewind':
          onRewind?.();
          return true;
        default:
          return false;
      }
    },
    [handleNewChat, clearMessages, activeSessionId, loadMessages, setActiveView, setDiagOpen, setObsOpen, addCompactPoint, setOp, onRewind],
  );

  return {
    handleSend,
    handleEditMessage,
    handleUndoMessage,
    handleCancelEdit,
    handleRestoreBranch,
    handleResendMessage,
    handleClearSession,
    handleNewChat,
    handleNewChatInWorkspace,
    handleDeleteSession,
    handleCreateWorkspace,
    handleCommand,
  };
}
