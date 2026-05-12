// ---------------------------------------------------------------------------
// useChatOperations -- send, cancel, edit, undo, resend, restore operations.
//
// Extracted from useChat.ts. These are the user-facing actions that mutate
// session state through the backend.
// ---------------------------------------------------------------------------

import { useCallback, useRef, type Dispatch, type SetStateAction } from 'react';
import { logger, transport } from '../lib';
import type {
  Message,
  ChatStarted,
  UndoResult,
  RestoreResult,
  ThinkingEffort,
  PlanMode,
  OperationMode,
  RequestMode,
} from '../types';
import { chatBusState } from './chatBus';
import {
  getCachedMessages,
  setCachedMessages,
  withSessionLock,
  findCheckpointForMessage,
} from './chatHelpers';
import type { ChatSharedRefs } from './chatSharedState';
import type { ChatOpStatus, PendingEdit, SendMessageOptions } from './useChat';

export interface UseChatOperationsReturn {
  sendMessage: (opts: SendMessageOptions) => Promise<ChatStarted | null>;
  cancelRun: () => Promise<void>;
  editMessage: (messageId: string, content: string) => void;
  cancelEdit: () => void;
  editAndResend: (
    sessionId: string,
    newContent: string,
    providerId?: string,
    thinkingEffort?: ThinkingEffort | null,
    planMode?: PlanMode,
    operationMode?: OperationMode,
    requestMode?: RequestMode,
  ) => Promise<ChatStarted | null>;
  undoToMessage: (
    sessionId: string,
    messageId: string,
  ) => Promise<UndoResult | null>;
  resendLastTurn: (
    sessionId: string,
    messageId: string,
    content: string,
    providerId?: string,
    thinkingEffort?: ThinkingEffort | null,
    planMode?: PlanMode,
  ) => Promise<ChatStarted | null>;
  restoreBranch: (
    sessionId: string,
    checkpointId: string,
  ) => Promise<RestoreResult | null>;
}

export function useChatOperations(
  refs: ChatSharedRefs,
  setOp: (status: ChatOpStatus) => void,
  setError: Dispatch<SetStateAction<string | null>>,
  syncVisible: (sessionId: string) => void,
  loadMessages: (sessionId: string) => Promise<void>,
  invalidateStaleContextResets: (sessionId: string, newMsgCount: number) => void,
  markSessionActivity: (sessionId: string, at?: number) => void,
  pendingEdit: PendingEdit | null,
  setPendingEdit: Dispatch<SetStateAction<PendingEdit | null>>,
  setStreamingSessionIds: Dispatch<SetStateAction<Set<string>>>,
  getRequestModeFromMessage: (message: { metadata?: Record<string, unknown> } | undefined) => RequestMode,
): UseChatOperationsReturn {
  // Synchronous guard for sendMessage -- prevents concurrent sends.
  const sendingRef = useRef(false);

  // ------------------------------------------------------------------
  // cancelRun
  // ------------------------------------------------------------------

  const cancelRun = useCallback(async () => {
    let runId: string | null = null;
    const sessionId = refs.activeSessionIdRef.current;
    if (sessionId) {
      runId =
        [...chatBusState.pendingRuns].find(
          (rid) => chatBusState.runToSession[rid] === sessionId,
        ) ?? null;
    }
    if (!runId) {
      logger.warn('[chat] cancelRun: no active run found');
      return;
    }
    try {
      await transport.invoke('chat_cancel', { runId });
    } catch (e) {
      logger.error('[chat] chat_cancel failed:', e);
    }
  }, [refs.activeSessionIdRef]);

  // ------------------------------------------------------------------
  // sendMessage
  // ------------------------------------------------------------------

  const sendMessage = useCallback(
    async (opts: SendMessageOptions): Promise<ChatStarted | null> => {
      const {
        message,
        sessionId,
        providerId,
        skills,
        knowledgeCollections,
        thinkingEffort,
        attachments,
        planMode,
        operationMode,
        mcpMode,
        mcpServers,
        requestMode,
        imageGenerationOptions,
      } = opts;
      if (refs.opStatusRef.current !== 'idle') {
        logger.warn(
          `[chat] sendMessage blocked: opStatus=${refs.opStatusRef.current}`,
        );
        return null;
      }
      if (sendingRef.current) {
        logger.warn('[chat] sendMessage blocked: already sending (ref guard)');
        return null;
      }
      sendingRef.current = true;

      setError(null);
      setOp('sending');
      refs.activeSessionIdRef.current = sessionId;
      markSessionActivity(sessionId);

      // Optimistic user message.
      const userMsg: Message = {
        id: `user-${Date.now()}`,
        role: 'user' as const,
        content: message,
        timestamp: new Date().toISOString(),
        tool_calls: [],
        skills: skills && skills.length > 0 ? skills : undefined,
        metadata:
          attachments && attachments.length > 0 ? { attachments } : undefined,
      };
      setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) => [
        ...prev,
        userMsg,
      ]);
      syncVisible(sessionId);

      try {
        const resetPoints =
          refs.contextResetMapRef.current.get(sessionId) ?? [];
        const resetIdx =
          resetPoints.length > 0 ? resetPoints[resetPoints.length - 1] : null;
        logger.debug(
          '[chat] sendMessage: planMode =',
          planMode,
          '-> sending:',
          planMode ?? null,
        );
        const result = await transport.invoke<ChatStarted>('chat_send', {
          message,
          sessionId,
          providerId: providerId ?? null,
          requestMode: requestMode ?? 'text_chat',
          skills: skills && skills.length > 0 ? skills : null,
          knowledgeCollections:
            knowledgeCollections && knowledgeCollections.length > 0
              ? knowledgeCollections
              : null,
          contextStartIndex: resetIdx,
          thinkingEffort: thinkingEffort ?? null,
          attachments:
            attachments && attachments.length > 0 ? attachments : null,
          planMode: planMode ?? null,
          operationMode: operationMode ?? null,
          mcpMode: mcpMode ?? null,
          mcpServers:
            mcpServers && mcpServers.length > 0 ? mcpServers : null,
          imageGenerationOptions: imageGenerationOptions ?? null,
        });        return result;
      } catch (e) {
        setError(String(e));
        chatBusState.streamingSessions.delete(sessionId);
        setStreamingSessionIds(new Set(chatBusState.streamingSessions));
        setOp('idle');

        // Rollback: remove the optimistic message on failure.
        setCachedMessages(refs.sessionMessagesRef.current, sessionId, (prev) =>
          prev.filter((m) => m.id !== userMsg.id),
        );
        syncVisible(sessionId);
        return null;
      } finally {
        sendingRef.current = false;
      }
    },
    [
      refs.opStatusRef,
      refs.activeSessionIdRef,
      refs.sessionMessagesRef,
      refs.contextResetMapRef,
      markSessionActivity,
      syncVisible,
      setOp,
      setError,
      setStreamingSessionIds,
    ],
  );

  // ------------------------------------------------------------------
  // editMessage / cancelEdit
  // ------------------------------------------------------------------

  const editMessage = useCallback((messageId: string, content: string) => {
    setPendingEdit({ messageId, content });
  }, [setPendingEdit]);

  const cancelEdit = useCallback(() => {
    setPendingEdit(null);
  }, [setPendingEdit]);

  // ------------------------------------------------------------------
  // editAndResend
  // ------------------------------------------------------------------

  const editAndResend = useCallback(
    async (
      sessionId: string,
      newContent: string,
      providerId?: string,
      thinkingEffort?: ThinkingEffort | null,
      planMode?: PlanMode,
      operationMode?: OperationMode,
      requestMode?: RequestMode,
    ): Promise<ChatStarted | null> => {
      if (refs.opStatusRef.current !== 'idle') {
        logger.warn(
          `[chat] editAndResend blocked: opStatus=${refs.opStatusRef.current}`,
        );
        return null;
      }

      const edit = pendingEdit;
      if (!edit) {
        logger.warn('[chat] editAndResend called without pending edit');
        return null;
      }

      setOp('editing');
      setError(null);
      markSessionActivity(sessionId);

      return withSessionLock(sessionId, async () => {
        try {
          // 1. Find checkpoint for the edited message.
          const checkpoint = await findCheckpointForMessage(
            sessionId,
            edit.messageId,
            refs.sessionMessagesRef.current,
          );

          if (!checkpoint) {
            logger.warn(
              '[chat] No checkpoint found for edit -- using truncate fallback',
            );
            const freshMsgs = await transport.invoke<Message[]>(
              'session_get_messages',
              { sessionId },
            );
            let userIdx = freshMsgs.findIndex((m) => m.id === edit.messageId);
            if (userIdx < 0) {
              const cachedMsg = getCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
              ).find((m) => m.id === edit.messageId);
              if (cachedMsg) {
                userIdx = freshMsgs.findIndex(
                  (m) =>
                    m.role === cachedMsg.role && m.content === cachedMsg.content,
                );
              }
            }

            if (userIdx >= 0) {
              await transport.invoke('session_truncate_messages', {
                sessionId,
                keepCount: userIdx,
              });
              const keptMsgs = freshMsgs.slice(0, userIdx);
              setCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
                keptMsgs,
              );
              invalidateStaleContextResets(sessionId, keptMsgs.length);
              syncVisible(sessionId);
              setPendingEdit(null);

              const userMsg: Message = {
                id: `user-${Date.now()}`,
                role: 'user' as const,
                content: newContent,
                timestamp: new Date().toISOString(),
                tool_calls: [],
              };
              setCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
                (prev) => [...prev, userMsg],
              );
              syncVisible(sessionId);

              const result = await transport.invoke<ChatStarted>('chat_send', {
                message: newContent,
                sessionId,
                providerId: providerId ?? null,
                requestMode: requestMode ?? 'text_chat',
                thinkingEffort: thinkingEffort ?? null,
                planMode: planMode ?? null,
                operationMode: operationMode ?? null,
              });
              return result;
            }

            logger.warn(
              '[chat] editAndResend: fallback failed, user message not found',
            );
            setPendingEdit(null);
            setOp('idle');
            return null;
          }

          // 2. Backend undo.
          await transport.invoke<UndoResult>('chat_undo', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
          });

          // 3. Reload messages.
          const msgs = await transport.invoke<Message[]>(
            'session_get_messages',
            { sessionId },
          );
          setCachedMessages(refs.sessionMessagesRef.current, sessionId, msgs);
          invalidateStaleContextResets(sessionId, msgs.length);
          syncVisible(sessionId);

          // 4. Clear edit state.
          setPendingEdit(null);

          // 5. Send new content.
          const userMsg: Message = {
            id: `user-${Date.now()}`,
            role: 'user' as const,
            content: newContent,
            timestamp: new Date().toISOString(),
            tool_calls: [],
          };
          setCachedMessages(
            refs.sessionMessagesRef.current,
            sessionId,
            (prev) => [...prev, userMsg],
          );
          syncVisible(sessionId);

          const result = await transport.invoke<ChatStarted>('chat_send', {
            message: newContent,
            sessionId,
            providerId: providerId ?? null,
            requestMode: requestMode ?? 'text_chat',
            thinkingEffort: thinkingEffort ?? null,
            planMode: planMode ?? null,
            operationMode: operationMode ?? null,
          });
          return result;
        } catch (e) {
          logger.error('[chat] editAndResend failed:', e);
          setError(String(e));
          setOp('idle');
          await loadMessages(sessionId);
          setPendingEdit(null);
          return null;
        }
      });
    },
    [
      refs.opStatusRef,
      refs.sessionMessagesRef,
      pendingEdit,
      syncVisible,
      setOp,
      setError,
      loadMessages,
      invalidateStaleContextResets,
      markSessionActivity,
      setPendingEdit,
    ],
  );

  // ------------------------------------------------------------------
  // undoToMessage
  // ------------------------------------------------------------------

  const undoToMessage = useCallback(
    async (
      sessionId: string,
      messageId: string,
    ): Promise<UndoResult | null> => {
      logger.debug(
        `[chat] undoToMessage called, opStatus=${refs.opStatusRef.current}, sessionId=${sessionId}, messageId=${messageId}`,
      );
      if (refs.opStatusRef.current !== 'idle') {
        logger.warn(
          `[chat] undoToMessage blocked: opStatus=${refs.opStatusRef.current}`,
        );
        return null;
      }

      setOp('undoing');
      setError(null);
      markSessionActivity(sessionId);

      return withSessionLock(sessionId, async () => {
        try {
          logger.debug(
            '[chat] undoToMessage: finding checkpoint for message...',
          );
          const checkpoint = await findCheckpointForMessage(
            sessionId,
            messageId,
            refs.sessionMessagesRef.current,
          );
          logger.debug('[chat] undoToMessage: checkpoint result', checkpoint);
          if (!checkpoint) {
            logger.warn(
              '[chat] No checkpoint found for message',
              messageId,
              '-- trying direct truncation fallback',
            );
            const backendMsgs = await transport.invoke<Message[]>(
              'session_get_messages',
              { sessionId },
            );
            let targetIdx = backendMsgs.findIndex((m) => m.id === messageId);
            if (targetIdx < 0) {
              const cached = refs.sessionMessagesRef.current
                .get(sessionId)
                ?.find((cm) => cm.id === messageId);
              if (cached) {
                targetIdx = backendMsgs.findIndex(
                  (m) => m.role === cached.role && m.content === cached.content,
                );
              }
            }
            if (targetIdx >= 0) {
              await transport.invoke('session_truncate_messages', {
                sessionId,
                keepCount: targetIdx,
              });
              invalidateStaleContextResets(sessionId, targetIdx);
              await loadMessages(sessionId);
              setOp('idle');
              return {
                messages_removed: targetIdx,
                restored_turn_number: 0,
                files_restored: 0,
              } as UndoResult;
            }
            logger.warn(
              '[chat] undoToMessage: fallback truncation also failed',
            );
            setOp('idle');
            return null;
          }

          logger.debug(
            `[chat] undoToMessage: calling chat_undo with checkpoint=${checkpoint.checkpoint_id}`,
          );
          const result = await transport.invoke<UndoResult>('chat_undo', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
          });
          logger.debug('[chat] undoToMessage: chat_undo result', result);

          logger.debug('[chat] undoToMessage: reloading messages...');
          invalidateStaleContextResets(
            sessionId,
            checkpoint.message_count_before,
          );
          await loadMessages(sessionId);
          logger.debug('[chat] undoToMessage: loadMessages complete');
          setOp('idle');
          return result;
        } catch (e) {
          logger.error('[chat] undoToMessage failed:', e);
          setError(String(e));
          setOp('idle');
          await loadMessages(sessionId);
          return null;
        }
      });
    },
    [
      refs.opStatusRef,
      refs.sessionMessagesRef,
      loadMessages,
      setOp,
      setError,
      invalidateStaleContextResets,
      markSessionActivity,
    ],
  );

  // ------------------------------------------------------------------
  // resendLastTurn
  // ------------------------------------------------------------------

  const resendLastTurn = useCallback(
    async (
      sessionId: string,
      messageId: string,
      content: string,
      providerId?: string,
      thinkingEffort?: ThinkingEffort | null,
      planMode?: PlanMode,
    ): Promise<ChatStarted | null> => {
      logger.debug(
        `[chat] resendLastTurn called, opStatus=${refs.opStatusRef.current}, sessionId=${sessionId}, messageId=${messageId}`,
      );
      if (refs.opStatusRef.current !== 'idle') {
        logger.warn(
          `[chat] resendLastTurn blocked: opStatus=${refs.opStatusRef.current}`,
        );
        return null;
      }

      setOp('resending');
      setError(null);
      markSessionActivity(sessionId);

      return withSessionLock(sessionId, async () => {
        try {
          // 0. Reload messages from backend.
          const freshMsgs = await transport.invoke<Message[]>(
            'session_get_messages',
            { sessionId },
          );
          setCachedMessages(
            refs.sessionMessagesRef.current,
            sessionId,
            freshMsgs,
          );

          // 1. Find checkpoint.
          const checkpoint = await findCheckpointForMessage(
            sessionId,
            messageId,
            refs.sessionMessagesRef.current,
          );
          logger.debug(
            '[chat] resendLastTurn: checkpoint result',
            checkpoint,
          );

          if (!checkpoint) {
            logger.warn(
              '[chat] No checkpoint found for resend -- using direct re-send fallback',
            );
            let userIdx = freshMsgs.findIndex((m) => m.id === messageId);
            if (userIdx < 0) {
              userIdx = freshMsgs.findIndex(
                (m) => m.role === 'user' && m.content === content,
              );
            }
            if (userIdx >= 0) {
              await transport.invoke('session_truncate_messages', {
                sessionId,
                keepCount: userIdx,
              });
              const keptMsgs = freshMsgs.slice(0, userIdx);
              setCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
                keptMsgs,
              );
              invalidateStaleContextResets(sessionId, keptMsgs.length);
              syncVisible(sessionId);

              const userMsg: Message = {
                id: `user-${Date.now()}`,
                role: 'user' as const,
                content,
                timestamp: new Date().toISOString(),
                tool_calls: [],
              };
              setCachedMessages(
                refs.sessionMessagesRef.current,
                sessionId,
                (prev) => [...prev, userMsg],
              );
              syncVisible(sessionId);

              const result = await transport.invoke<ChatStarted>('chat_send', {
                message: content,
                sessionId,
                providerId: providerId ?? null,
                requestMode: getRequestModeFromMessage(freshMsgs[userIdx]),
                thinkingEffort: thinkingEffort ?? null,
                planMode: planMode ?? null,
              });
              logger.debug(
                '[chat] resendLastTurn: fallback chat_send result',
                result,
              );
              return result;
            }
            logger.warn(
              '[chat] resendLastTurn: fallback also failed, user message not found',
            );
            setOp('idle');
            return null;
          }

          // 2. Remove the assistant reply from the cache.
          const keepCount = checkpoint.message_count_before + 1;
          const keptMsgs = freshMsgs.slice(0, keepCount);
          const removedMsgs = freshMsgs.slice(keepCount);
          logger.debug(
            `[chat] resendLastTurn: checkpoint.message_count_before=${checkpoint.message_count_before}, keepCount=${keepCount}, total=${freshMsgs.length}`,
          );
          logger.debug(
            '[chat] resendLastTurn: keeping:',
            keptMsgs.map((m) => `[${m.role}] ${m.content.slice(0, 40)}...`),
          );
          logger.debug(
            '[chat] resendLastTurn: removing:',
            removedMsgs.map((m) => `[${m.role}] ${m.content.slice(0, 40)}...`),
          );
          setCachedMessages(
            refs.sessionMessagesRef.current,
            sessionId,
            keptMsgs,
          );
          invalidateStaleContextResets(sessionId, keptMsgs.length);
          syncVisible(sessionId);

          // 3. Backend resend.
          logger.debug(
            `[chat] resendLastTurn: calling chat_resend with checkpoint=${checkpoint.checkpoint_id}`,
          );
          const result = await transport.invoke<ChatStarted>('chat_resend', {
            sessionId,
            checkpointId: checkpoint.checkpoint_id,
            providerId: providerId ?? null,
            requestMode: getRequestModeFromMessage(
              keptMsgs[keptMsgs.length - 1],
            ),
            thinkingEffort: thinkingEffort ?? null,
            planMode: planMode ?? null,
          });
          logger.debug(
            '[chat] resendLastTurn: chat_resend result',
            result,
          );
          return result;
        } catch (e) {
          logger.error('[chat] resendLastTurn failed:', e);
          setError(String(e));
          setOp('idle');
          await loadMessages(sessionId);
          return null;
        }
      });
    },
    [
      refs.opStatusRef,
      refs.sessionMessagesRef,
      syncVisible,
      setOp,
      setError,
      loadMessages,
      invalidateStaleContextResets,
      markSessionActivity,
      getRequestModeFromMessage,
    ],
  );

  // ------------------------------------------------------------------
  // restoreBranch
  // ------------------------------------------------------------------

  const restoreBranch = useCallback(
    async (
      sessionId: string,
      checkpointId: string,
    ): Promise<RestoreResult | null> => {
      if (refs.opStatusRef.current !== 'idle') {
        logger.warn(
          `[chat] restoreBranch blocked: opStatus=${refs.opStatusRef.current}`,
        );
        return null;
      }

      setOp('restoring');
      setError(null);
      markSessionActivity(sessionId);

      return withSessionLock(sessionId, async () => {
        try {
          const result = await transport.invoke<RestoreResult>(
            'chat_restore_branch',
            {
              sessionId,
              checkpointId,
            },
          );
          await loadMessages(sessionId);
          setOp('idle');
          return result;
        } catch (e) {
          logger.error('[chat] restoreBranch failed:', e);
          setError(String(e));
          setOp('idle');
          return null;
        }
      });
    },
    [refs.opStatusRef, loadMessages, setOp, setError, markSessionActivity],
  );

  return {
    sendMessage,
    cancelRun,
    editMessage,
    cancelEdit,
    editAndResend,
    undoToMessage,
    resendLastTurn,
    restoreBranch,
  };
}
