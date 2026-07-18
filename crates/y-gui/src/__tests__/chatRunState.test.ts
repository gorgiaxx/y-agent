import { describe, expect, it } from 'vitest';

import {
  applyAwaitingInteraction,
  applyInteractionResolved,
  applyRunStarted,
  applyRunTerminal,
  createChatRunState,
  getPendingRunIdForSession,
  getTerminalRunContext,
  hasPendingRunForSession,
  isPlanResumeRun,
  markSubSessionStreaming,
} from '../hooks/chatRunState';

describe('chatRunState', () => {
  it('keeps a session active when an older run finishes after a newer run already started', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-old', 'session-1');
    state = applyRunStarted(state, 'run-new', 'session-1');
    state = applyRunTerminal(state, 'run-old', 'session-1');

    expect(hasPendingRunForSession(state, 'session-1')).toBe(true);
    expect(state.streamingSessions.has('session-1')).toBe(true);
  });

  it('clears the session only when the last pending run for that session finishes', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-old', 'session-1');
    state = applyRunStarted(state, 'run-new', 'session-1');
    state = applyRunTerminal(state, 'run-old', 'session-1');
    state = applyRunTerminal(state, 'run-new', 'session-1');

    expect(hasPendingRunForSession(state, 'session-1')).toBe(false);
    expect(state.streamingSessions.has('session-1')).toBe(false);
  });

  it('can recover the active run id for a session even if the streaming marker was lost', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-1', 'session-1');
    state = {
      ...state,
      streamingSessions: new Set(),
    };

    expect(getPendingRunIdForSession(state, 'session-1')).toBe('run-1');
    expect(hasPendingRunForSession(state, 'session-1')).toBe(true);
    expect(state.streamingSessions.has('session-1')).toBe(false);
  });

  it('keeps a session running while a plan review waits for user approval', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-1', 'session-1');
    state = applyAwaitingInteraction(state, 'run-1', 'session-1');
    state = {
      ...state,
      streamingSessions: new Set(),
    };

    expect(hasPendingRunForSession(state, 'session-1')).toBe(true);
    expect(state.awaitingInteractionRuns.has('run-1')).toBe(true);
  });

  it('restores running state when a pending plan review is approved', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-1', 'session-1');
    state = applyAwaitingInteraction(state, 'run-1', 'session-1');
    state = {
      ...state,
      streamingSessions: new Set(),
    };
    state = applyInteractionResolved(state, 'run-1', 'session-1');

    expect(state.awaitingInteractionRuns.has('run-1')).toBe(false);
    expect(state.streamingSessions.has('session-1')).toBe(true);
  });

  it('removes runToSession entry on terminal so callers must resolve session before applying', () => {
    let state = createChatRunState();

    state = applyRunStarted(state, 'run-1', 'session-1');
    expect(state.runToSession['run-1']).toBe('session-1');

    const resolvedBefore = state.runToSession['run-1'];
    state = applyRunTerminal(state, 'run-1', '');

    expect(state.runToSession['run-1']).toBeUndefined();
    expect(resolvedBefore).toBe('session-1');
  });

  it('marks a child session as streaming so the drill-in sub-chat shows running', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-1', 'parent-1');
    state = markSubSessionStreaming(state, 'child-1');

    expect(state.streamingSessions.has('child-1')).toBe(true);
    expect(state.streamingChildSessions.has('child-1')).toBe(true);
    expect(state.streamingSessions.has('parent-1')).toBe(true);
  });

  it('cleans up child sessions from streaming when the parent run ends', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-1', 'parent-1');
    state = markSubSessionStreaming(state, 'child-1');
    state = markSubSessionStreaming(state, 'child-2');

    expect(state.streamingSessions.has('child-1')).toBe(true);
    expect(state.streamingSessions.has('child-2')).toBe(true);

    state = applyRunTerminal(state, 'run-1', 'parent-1');

    expect(state.streamingSessions.has('parent-1')).toBe(false);
    expect(state.streamingSessions.has('child-1')).toBe(false);
    expect(state.streamingSessions.has('child-2')).toBe(false);
    expect(state.streamingChildSessions.size).toBe(0);
  });

  it('does not clear child sessions if the parent still has pending runs', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-1', 'parent-1');
    state = applyRunStarted(state, 'run-2', 'parent-1');
    state = markSubSessionStreaming(state, 'child-1');

    state = applyRunTerminal(state, 'run-1', 'parent-1');

    expect(state.streamingSessions.has('parent-1')).toBe(true);
    expect(state.streamingSessions.has('child-1')).toBe(true);
  });

  it('tracks plan_resume kind so callers can distinguish background retries', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-chat', 'session-1', 'chat');
    state = applyRunStarted(state, 'run-resume', 'session-1', 'plan_resume');

    expect(isPlanResumeRun(state, 'run-chat')).toBe(false);
    expect(isPlanResumeRun(state, 'run-resume')).toBe(true);
    // Absent kind defaults to chat (not plan_resume).
    state = applyRunStarted(state, 'run-plain', 'session-1');
    expect(isPlanResumeRun(state, 'run-plain')).toBe(false);
  });

  it('preserves background auto-wake kind without treating it as plan resume', () => {
    let state = createChatRunState();
    state = applyRunStarted(
      state,
      'run-wake',
      'session-1',
      'background_auto_wake',
    );

    expect(isPlanResumeRun(state, 'run-wake')).toBe(false);
    expect(getTerminalRunContext(state, 'run-wake')).toEqual({
      sessionId: 'session-1',
      kind: 'background_auto_wake',
    });
  });

  it('clears runKinds entry when the run terminates', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-resume', 'session-1', 'plan_resume');
    expect(isPlanResumeRun(state, 'run-resume')).toBe(true);

    state = applyRunTerminal(state, 'run-resume', 'session-1');
    expect(isPlanResumeRun(state, 'run-resume')).toBe(false);
    expect(state.runKinds['run-resume']).toBeUndefined();
  });

  it('captures terminal run kind before terminal cleanup removes it', () => {
    let state = createChatRunState();
    state = applyRunStarted(state, 'run-resume', 'session-1', 'plan_resume');

    expect(getTerminalRunContext(state, 'run-resume', '')).toEqual({
      sessionId: 'session-1',
      kind: 'plan_resume',
    });

    state = applyRunTerminal(state, 'run-resume', 'session-1');
    expect(state.runKinds['run-resume']).toBeUndefined();
  });
 });
