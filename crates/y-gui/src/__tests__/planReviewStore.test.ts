import { describe, expect, it } from 'vitest';

import {
  addPlanReview,
  clearPlanReview,
  getPendingReviewIdsForSession,
  type PlanReviewStore,
} from '../utils/planReviewStore';

function entry(reviewId: string, runId: string, sessionId: string) {
  return { reviewId, runId, sessionId, plan: {} };
}

describe('planReviewStore', () => {
  it('keeps multiple pending reviews for the same session', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-1', 'sess-a'));

    const ids = getPendingReviewIdsForSession(store, 'sess-a');
    expect(ids.has('rev-1')).toBe(true);
    expect(ids.has('rev-2')).toBe(true);
    expect(ids.size).toBe(2);
  });

  it('scopes pending review ids to the requested session', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-2', 'sess-b'));

    expect(getPendingReviewIdsForSession(store, 'sess-a')).toEqual(new Set(['rev-1']));
    expect(getPendingReviewIdsForSession(store, 'sess-b')).toEqual(new Set(['rev-2']));
    expect(getPendingReviewIdsForSession(store, null)).toEqual(new Set());
  });

  it('clears only the answered review and leaves siblings actionable', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-1', 'sess-a'));

    const { store: next } = clearPlanReview(store, 'rev-1');

    expect(getPendingReviewIdsForSession(next, 'sess-a')).toEqual(new Set(['rev-2']));
  });

  it('does not resume the run while sibling reviews on the same run remain', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-1', 'sess-a'));

    const { resolvedRun } = clearPlanReview(store, 'rev-1');

    expect(resolvedRun).toBeNull();
  });

  it('resumes the run only when the last review for that run is cleared', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-1', 'sess-a'));

    const afterFirst = clearPlanReview(store, 'rev-1');
    const afterSecond = clearPlanReview(afterFirst.store, 'rev-2');

    expect(afterSecond.resolvedRun).toEqual({ runId: 'run-1', sessionId: 'sess-a' });
  });

  it('resumes each run independently when reviews span different runs', () => {
    let store: PlanReviewStore = {};
    store = addPlanReview(store, entry('rev-1', 'run-1', 'sess-a'));
    store = addPlanReview(store, entry('rev-2', 'run-2', 'sess-a'));

    const { resolvedRun } = clearPlanReview(store, 'rev-1');

    expect(resolvedRun).toEqual({ runId: 'run-1', sessionId: 'sess-a' });
  });

  it('is a no-op when clearing an unknown review id', () => {
    const store = addPlanReview({}, entry('rev-1', 'run-1', 'sess-a'));
    const { store: next, resolvedRun } = clearPlanReview(store, 'missing');

    expect(next).toBe(store);
    expect(resolvedRun).toBeNull();
  });
});
