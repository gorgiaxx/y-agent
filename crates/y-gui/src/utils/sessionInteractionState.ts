export type SessionInteractionMap<T> = Record<string, T>;

export function getSessionInteraction<T>(
  state: SessionInteractionMap<T>,
  sessionId: string | null | undefined,
): T | null {
  if (!sessionId) {
    return null;
  }
  return state[sessionId] ?? null;
}

export function setSessionInteraction<T>(
  state: SessionInteractionMap<T>,
  sessionId: string,
  value: T,
): SessionInteractionMap<T> {
  return {
    ...state,
    [sessionId]: value,
  };
}

export function clearSessionInteractionByPredicate<T>(
  state: SessionInteractionMap<T>,
  predicate: (value: T) => boolean,
): SessionInteractionMap<T> {
  const sessionId = Object.keys(state).find((key) => predicate(state[key]));
  if (!sessionId) {
    return state;
  }

  const next = { ...state };
  delete next[sessionId];
  return next;
}
