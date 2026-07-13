export interface ChatRunState {
  runToSession: Record<string, string>;
  streamingSessions: Set<string>;
  pendingRuns: Set<string>;
  awaitingInteractionRuns: Set<string>;
  /** Child (sub-agent) session ids that received streaming events during an
   *  active parent run. Tracked separately so they can be cleaned up from
   *  `streamingSessions` when the parent run terminates (the parent is the
   *  only entry `applyRunTerminal` knows about from `runToSession`). */
  streamingChildSessions: Set<string>;
  /** Run kind per run_id: `chat` (normal LLM turn) or `plan_resume`
   *  (background plan-execution retry that must not create a new assistant
   *  bubble). Absent == `chat`. */
  runKinds: Record<string, string>;
}

export function createChatRunState(): ChatRunState {
  return {
    runToSession: {},
    streamingSessions: new Set(),
    pendingRuns: new Set(),
    awaitingInteractionRuns: new Set(),
    streamingChildSessions: new Set(),
    runKinds: {},
  };
}

export function hasPendingRunForSession(
  state: ChatRunState,
  sessionId: string,
): boolean {
  return getPendingRunIdForSession(state, sessionId) !== null;
}

export function hasAwaitingInteractionForSession(
  state: ChatRunState,
  sessionId: string,
): boolean {
  for (const runId of state.awaitingInteractionRuns) {
    if (state.runToSession[runId] === sessionId) {
      return true;
    }
  }

  return false;
}

export function getPendingRunIdForSession(
  state: ChatRunState,
  sessionId: string,
): string | null {
  for (const runId of state.pendingRuns) {
    if (state.runToSession[runId] === sessionId) {
      return runId;
    }
  }

  return null;
}

export function applyRunStarted(
  state: ChatRunState,
  runId: string,
  sessionId: string,
  kind?: string,
): ChatRunState {
  const pendingRuns = new Set(state.pendingRuns);
  pendingRuns.add(runId);

  const awaitingInteractionRuns = new Set(state.awaitingInteractionRuns);
  awaitingInteractionRuns.delete(runId);

  const streamingSessions = new Set(state.streamingSessions);
  streamingSessions.add(sessionId);

  const runKinds = kind && kind !== 'chat'
    ? { ...state.runKinds, [runId]: kind }
    : state.runKinds;

  return {
    runToSession: {
      ...state.runToSession,
      [runId]: sessionId,
    },
    pendingRuns,
    streamingSessions,
    awaitingInteractionRuns,
    streamingChildSessions: state.streamingChildSessions,
    runKinds,
  };
}

/// Whether `runId` is a background plan-execution retry (`plan_resume`).
/// Such runs update the existing Plan card in-place without creating a new
/// assistant bubble or reloading messages on completion.
export function isPlanResumeRun(state: ChatRunState, runId: string): boolean {
  return state.runKinds[runId] === 'plan_resume';
}

export function getTerminalRunContext(
  state: ChatRunState,
  runId: string,
  explicitSessionId?: string,
): { sessionId: string; kind?: string } {
  return {
    sessionId: explicitSessionId || state.runToSession[runId] || '',
    ...(state.runKinds[runId] ? { kind: state.runKinds[runId] } : {}),
  };
}

/// Mark a child (sub-agent) session as actively streaming so the drill-in
/// sub-chat's input area reflects the running state. Called when a sub-session
/// streaming event arrives during a parent run.
export function markSubSessionStreaming(
  state: ChatRunState,
  childSessionId: string,
): ChatRunState {
  if (state.streamingSessions.has(childSessionId)) return state;
  const streamingSessions = new Set(state.streamingSessions);
  streamingSessions.add(childSessionId);
  const streamingChildSessions = new Set(state.streamingChildSessions);
  streamingChildSessions.add(childSessionId);
  return {
    ...state,
    streamingSessions,
    streamingChildSessions,
  };
}

export function applyRunTerminal(
  state: ChatRunState,
  runId: string,
  explicitSessionId?: string,
): ChatRunState {
  const pendingRuns = new Set(state.pendingRuns);
  pendingRuns.delete(runId);

  const awaitingInteractionRuns = new Set(state.awaitingInteractionRuns);
  awaitingInteractionRuns.delete(runId);

  const sessionId = explicitSessionId || state.runToSession[runId];
  const streamingSessions = new Set(state.streamingSessions);

  if (sessionId) {
    const pendingForSession = [...pendingRuns].some(
      (pendingRunId) => state.runToSession[pendingRunId] === sessionId,
    );
    if (!pendingForSession) {
      streamingSessions.delete(sessionId);
    }
  }

  // When the parent session stops streaming (no remaining pending runs for
  // it), clear all child sessions that were streaming under it. They cannot
  // outlive the parent run.
  if (sessionId && !streamingSessions.has(sessionId)) {
    for (const childId of state.streamingChildSessions) {
      streamingSessions.delete(childId);
    }
  }

  const remainingRunToSession = Object.fromEntries(
    Object.entries(state.runToSession).filter(([key]) => key !== runId),
  );

  const streamingChildSessions = streamingSessions.has(sessionId ?? '')
    ? state.streamingChildSessions
    : new Set<string>();

  const remainingRunKinds = { ...state.runKinds };
  delete remainingRunKinds[runId];

  return {
    runToSession: remainingRunToSession,
    pendingRuns,
    streamingSessions,
    awaitingInteractionRuns,
    streamingChildSessions,
    runKinds: remainingRunKinds,
  };
}

export function applyAwaitingInteraction(
  state: ChatRunState,
  runId: string,
  sessionId: string,
): ChatRunState {
  const pendingRuns = new Set(state.pendingRuns);
  pendingRuns.add(runId);

  const awaitingInteractionRuns = new Set(state.awaitingInteractionRuns);
  awaitingInteractionRuns.add(runId);

  const streamingSessions = new Set(state.streamingSessions);
  streamingSessions.add(sessionId);

  return {
    runToSession: {
      ...state.runToSession,
      [runId]: sessionId,
    },
    pendingRuns,
    streamingSessions,
    awaitingInteractionRuns,
    streamingChildSessions: state.streamingChildSessions,
    runKinds: state.runKinds,
  };
}

export function applyInteractionResolved(
  state: ChatRunState,
  runId: string,
  sessionId: string,
): ChatRunState {
  const awaitingInteractionRuns = new Set(state.awaitingInteractionRuns);
  awaitingInteractionRuns.delete(runId);

  const streamingSessions = new Set(state.streamingSessions);
  if (state.pendingRuns.has(runId)) {
    streamingSessions.add(sessionId);
  }

  return {
    ...state,
    runToSession: {
      ...state.runToSession,
      [runId]: sessionId,
    },
    streamingSessions,
    awaitingInteractionRuns,
  };
}
