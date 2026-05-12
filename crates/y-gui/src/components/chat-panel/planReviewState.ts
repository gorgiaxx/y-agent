import { createContext, useContext } from 'react';

export interface PlanReviewState {
  reviewId: string | null;
  onApprove: (reviewId: string) => void;
  onRevise: (reviewId: string, feedback: string) => void;
  onReject: (reviewId: string, feedback: string) => void;
}

export const PlanReviewContext = createContext<PlanReviewState | null>(null);

export function usePlanReview(): PlanReviewState | null {
  return useContext(PlanReviewContext);
}
