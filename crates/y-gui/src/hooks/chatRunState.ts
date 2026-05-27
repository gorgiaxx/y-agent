export interface ChatRunState {
  runToSession: Record<string, string>;
  streamingSessions: Set<string>;
  pendingRuns: Set<string>;
  awaitingInteractionRuns: Set<string>;
}

export function createChatRunState(): ChatRunState {
  return {
    runToSession: {},
    streamingSessions: new Set(),
    pendingRuns: new Set(),
    awaitingInteractionRuns: new Set(),
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
): ChatRunState {
  const pendingRuns = new Set(state.pendingRuns);
  pendingRuns.add(runId);

  const awaitingInteractionRuns = new Set(state.awaitingInteractionRuns);
  awaitingInteractionRuns.delete(runId);

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

  const remainingRunToSession = Object.fromEntries(
    Object.entries(state.runToSession).filter(([key]) => key !== runId),
  );

  return {
    runToSession: remainingRunToSession,
    pendingRuns,
    streamingSessions,
    awaitingInteractionRuns,
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
