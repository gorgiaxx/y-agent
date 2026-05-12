import { useCallback, useEffect, useState } from 'react';
import { transport } from '../lib';

import {
  clearSessionInteractionById,
  getSessionInteraction,
  setSessionInteraction,
} from '../utils/sessionInteractionState';

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

export interface PlanReviewDialogState {
  reviewId: string;
  plan: Record<string, unknown>;
}

export function useSessionInteractions(activeSessionId: string | null) {
  const [askUserBySession, setAskUserBySession] = useState<Record<string, AskUserDialogState>>({});
  const [permissionBySession, setPermissionBySession] = useState<Record<string, PermissionDialogState>>({});
  const [planReviewBySession, setPlanReviewBySession] = useState<Record<string, PlanReviewDialogState>>({});

  const askUserData = getSessionInteraction(askUserBySession, activeSessionId);
  const permissionData = getSessionInteraction(permissionBySession, activeSessionId);
  const planReviewData = getSessionInteraction(planReviewBySession, activeSessionId);

  useEffect(() => {
    const unlisten = transport.listen<{
      run_id: string;
      session_id: string;
      interaction_id: string;
      questions: unknown;
    }>('chat:AskUser', (event) => {
      const { session_id, interaction_id, questions } = event.payload;
      setAskUserBySession((prev) => setSessionInteraction(prev, session_id, {
        interactionId: interaction_id,
        questions: questions as AskUserDialogState['questions'],
      }));
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const unlisten = transport.listen<{
      run_id: string;
      session_id: string;
      request_id: string;
      tool_name: string;
      action_description: string;
      reason: string;
      content_preview: string | null;
    }>('chat:PermissionRequest', (event) => {
      const { session_id, request_id, tool_name, action_description, reason, content_preview } = event.payload;
      setPermissionBySession((prev) => setSessionInteraction(prev, session_id, {
        requestId: request_id,
        toolName: tool_name,
        actionDescription: action_description,
        reason,
        contentPreview: content_preview,
      }));
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const unlisten = transport.listen<{
      run_id: string;
      session_id: string;
      review_id: string;
      plan: Record<string, unknown>;
    }>('chat:PlanReview', (event) => {
      const { session_id, review_id, plan } = event.payload;
      setPlanReviewBySession((prev) => setSessionInteraction(prev, session_id, {
        reviewId: review_id,
        plan,
      }));
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  const handleAskUserSubmit = useCallback((interactionId: string, answers: Record<string, string>) => {
    setAskUserBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.interactionId === interactionId,
    ));
    transport.invoke('chat_answer_question', {
      interactionId,
      answers: { answers },
    }).catch(console.error);
  }, []);

  const handleAskUserDismiss = useCallback((interactionId: string) => {
    setAskUserBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.interactionId === interactionId,
    ));
    transport.invoke('chat_answer_question', {
      interactionId,
      answers: { answers: {} },
    }).catch(console.error);
  }, []);

  const handlePermissionApprove = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'approve',
    }).catch(console.error);
  }, []);

  const handlePermissionDeny = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'deny',
    }).catch(console.error);
  }, []);

  const handlePermissionAllowAllForSession = useCallback((requestId: string) => {
    setPermissionBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.requestId === requestId,
    ));
    transport.invoke('chat_answer_permission', {
      requestId,
      decision: 'allow_all_for_session',
    }).catch(console.error);
  }, []);

  const handlePlanReviewApprove = useCallback((reviewId: string) => {
    setPlanReviewBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.reviewId === reviewId,
    ));
    transport.invoke('chat_answer_plan_review', {
      reviewId,
      decision: 'approve',
    }).catch(console.error);
  }, []);

  const handlePlanReviewReject = useCallback((reviewId: string, feedback: string) => {
    setPlanReviewBySession((prev) => clearSessionInteractionById(
      prev,
      (interaction) => interaction.reviewId === reviewId,
    ));
    transport.invoke('chat_answer_plan_review', {
      reviewId,
      decision: 'reject',
      feedback,
    }).catch(console.error);
  }, []);

  return {
    askUserData,
    permissionData,
    planReviewData,
    handleAskUserSubmit,
    handleAskUserDismiss,
    handlePermissionApprove,
    handlePermissionDeny,
    handlePermissionAllowAllForSession,
    handlePlanReviewApprove,
    handlePlanReviewReject,
  };
}
