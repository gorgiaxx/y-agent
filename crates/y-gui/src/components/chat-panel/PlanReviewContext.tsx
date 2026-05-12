import type { ReactNode } from 'react';
import { PlanReviewContext, type PlanReviewState } from './planReviewState';

export function PlanReviewProvider({
  value,
  children,
}: {
  value: PlanReviewState;
  children: ReactNode;
}) {
  return (
    <PlanReviewContext.Provider value={value}>
      {children}
    </PlanReviewContext.Provider>
  );
}
