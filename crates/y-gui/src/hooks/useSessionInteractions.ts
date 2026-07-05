import { useCallback, useEffect, useState } from 'react';
import { transport } from '../lib';
import {
  markChatRunAwaitingInteraction,
  resolveChatRunInteraction,
} from './chatBus';
import { useTransportListener } from './useTransportListener';

import {
  clearSessionInteractionByPredicate,
  getSessionInteraction,
  setSessionInteraction,
} from '../utils/sessionInteractionState';
import {
  addPlanReview,
  clearPlanReview,
  getPendingReviewIdsForSession,
  type PlanReviewStore,
} from '../utils/planReviewStore';

export interface AskUserDialogState {
  interactionId: string;
  questions: Array<{
    question: string;
    options: string[];
    multi_select?: boolean;
  }>;
}

export interface PermissionDialogState {
  requestId: string;
  toolName: string;
  actionDescription: string;
  reason: string;
  contentPreview: string | null;
}

export function useSessionInteractions(activeSessionId: string | null) {
  const [askUserBySession, setAskUserBySession] = useState<Record<string, AskUserDialogState>>({});
  const [permissionBySession, setPermissionBySession] = useState<Record<string, PermissionDialogState>>({});
  const [planReviewStore, setPlanReviewStore] = useState<PlanReviewStore>({});

  const askUserData = getSessionInteraction(askUserBySession, activeSessionId);
  const permissionData = getSessionInteraction(permissionBySession, activeSessionId);
  const pendingReviewIds = getPendingReviewIdsForSession(planReviewStore, activeSessionId);
  useTransportListener<{
    run_id: string;
    session_id: string;
    interaction_id: string;
    questions: unknown;
  }>(
    'chat:AskUser',
    (event) => {
      const { session_id, interaction_id, questions } = event.payload;
      setAskUserBySession((prev) => setSessionInteraction(prev, session_id, {
        interactionId: interaction_id,
        questions: questions as AskUserDialogState['questions'],
      }));
    },
    [],
  );

  useTransportListener<{
    run_id: string;
    session_id: string;
    request_id: string;
    tool_name: string;
    action_description: string;
    reason: string;
    content_preview: string | null;
  }>(
    'chat:PermissionRequest',
    (event) => {
      const { session_id, request_id, tool_name, action_description, reason, content_preview } = event.payload;
      setPermissionBySession((prev) => setSessionInteraction(prev, session_id, {
        requestId: request_id,
        toolName: tool_name,
        actionDescription: action_description,
        reason,
        contentPreview: content_preview,
      }));
    },
    [],
  );

  useTransportListener<{
    run_id: string;
    session_id: string;
    review_id: string;
    plan: Record<string, unknown>;
  }>(
    'chat:PlanReview',
    (event) => {
      const { run_id, session_id, review_id, plan } = event.payload;
      markChatRunAwaitingInteraction(run_id, session_id);
      setPlanReviewStore((prev) => addPlanReview(prev, {
        reviewId: review_id,
        runId: run_id,
        sessionId: session_id,
        plan,
      }));
    },
    [],
  );

  useEffect(() => {
    if (!activeSessionId) return;
    transport.invoke('session_restore_pending_reviews', {
      sessionId: activeSessionId,
    }).catch(() => {});
  }, [activeSessionId]);

  const handleAskUserSubmit = useCallback((interactionId: string, answers: Record<string, string>) => {
    setAskUserBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.interactionId === interactionId,
    ));
    transport.invoke('chat_answer_question', {
      interactionId,
      answers: { answers },
    }).catch(console.error);
  }, []);

  const handleAskUserDismiss = useCallback((interactionId: string) => {
    setAskUserBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.interactionId === interactionId,
    ));
    transport.invoke('chat_answer_question', {
      interactionId,
      answers: { answers: {} },
    }).catch(console.error);
  }, []);

  const handlePermissionApprove = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'approve',
    }).catch(console.error);
  }, []);

  const handlePermissionDeny = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'deny',
    }).catch(console.error);
  }, []);

  const handlePermissionAllowAllForSession = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'allow_all_for_session',
    }).catch(console.error);
  }, []);

  const handlePermissionApproveAlways = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionByPredicate(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'approve_always',
    }).catch(console.error);
  }, []);

  const answerPlanReview = useCallback((
    reviewId: string,
    decision: 'approve' | 'revise' | 'reject',
    feedback?: string,
  ) => {
    setPlanReviewStore((prev) => {
      const { store, resolvedRun } = clearPlanReview(prev, reviewId);
      if (resolvedRun) {
        resolveChatRunInteraction(resolvedRun.runId, resolvedRun.sessionId);
      }
      return store;
    });
    transport.invoke('chat_answer_plan_review', {
      reviewId,
      decision,
      ...(feedback !== undefined ? { feedback } : {}),
    }).catch(console.error);
  }, []);

  const handlePlanReviewApprove = useCallback((reviewId: string) => {
    answerPlanReview(reviewId, 'approve');
  }, [answerPlanReview]);

  const handlePlanReviewRevise = useCallback((reviewId: string, feedback: string) => {
    answerPlanReview(reviewId, 'revise', feedback);
  }, [answerPlanReview]);

  const handlePlanReviewReject = useCallback((reviewId: string, feedback: string) => {
    answerPlanReview(reviewId, 'reject', feedback);
  }, [answerPlanReview]);

  return {
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
  };
}
