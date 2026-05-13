export interface ChatRunState {
  runToSession: Record<string, string>;
  streamingSessions: Set<string>;
  pendingRuns: Set<string>;
}

export function createChatRunState(): ChatRunState {
  return {
    runToSession: {},
    streamingSessions: new Set(),
    pendingRuns: new Set(),
  };
}

export function hasPendingRunForSession(
  state: ChatRunState,
  sessionId: string,
): boolean {
  return getPendingRunIdForSession(state, sessionId) !== null;
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

  const streamingSessions = new Set(state.streamingSessions);
  streamingSessions.add(sessionId);

  return {
    runToSession: {
      ...state.runToSession,
      [runId]: sessionId,
    },
    pendingRuns,
    streamingSessions,
  };
}

export function applyRunTerminal(
  state: ChatRunState,
  runId: string,
  explicitSessionId?: string,
): ChatRunState {
  const pendingRuns = new Set(state.pendingRuns);
  pendingRuns.delete(runId);

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
  };
}
