import { describe, expect, it } from 'vitest';

import {
  applyRunStarted,
  applyRunTerminal,
  createChatRunState,
  getPendingRunIdForSession,
  hasPendingRunForSession,
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
});
