// ---------------------------------------------------------------------------
// Pure helpers for tracking multiple concurrent plan reviews.
//
// A single chat run (and a single session) can have more than one plan review
// awaiting the user at once: a top-level plan plus any reviews surfaced before
// approval. Keying by `reviewId` lets each plan bubble bind to its own review
// instead of a single shared slot, so approving one never disturbs another.
// ---------------------------------------------------------------------------

export interface PlanReviewEntry {
  reviewId: string;
  runId: string;
  sessionId: string;
  plan: Record<string, unknown>;
}

export type PlanReviewStore = Record<string, PlanReviewEntry>;

export interface ResolvedRun {
  runId: string;
  sessionId: string;
}

export interface ClearPlanReviewResult {
  store: PlanReviewStore;
  /// The run to resume, set only when no other reviews for that run remain.
  resolvedRun: ResolvedRun | null;
}

export function addPlanReview(
  store: PlanReviewStore,
  entry: PlanReviewEntry,
): PlanReviewStore {
  return { ...store, [entry.reviewId]: entry };
}

export function getPendingReviewIdsForSession(
  store: PlanReviewStore,
  sessionId: string | null | undefined,
): Set<string> {
  const ids = new Set<string>();
  if (!sessionId) {
    return ids;
  }
  for (const entry of Object.values(store)) {
    if (entry.sessionId === sessionId) {
      ids.add(entry.reviewId);
    }
  }
  return ids;
}

export function clearPlanReview(
  store: PlanReviewStore,
  reviewId: string,
): ClearPlanReviewResult {
  const cleared = store[reviewId];
  if (!cleared) {
    return { store, resolvedRun: null };
  }

  const next = { ...store };
  delete next[reviewId];

  const runStillHasReviews = Object.values(next).some(
    (entry) => entry.runId === cleared.runId,
  );

  return {
    store: next,
    resolvedRun: runStillHasReviews
      ? null
      : { runId: cleared.runId, sessionId: cleared.sessionId },
  };
}
