export interface SessionInputDraft {
  text: string;
  skills: string[];
}

interface StoredSessionInputState {
  draft?: SessionInputDraft;
  providerId?: string;
}

export interface ResolvedSessionInputState {
  draft: SessionInputDraft;
  providerId: string;
}

export type SessionInputStates = Record<string, StoredSessionInputState>;

export function createSessionInputStates(): SessionInputStates {
  return {};
}

export function getSessionInputState(
  states: SessionInputStates,
  sessionId: string,
  defaultProviderId: string,
): ResolvedSessionInputState {
  const stored = states[sessionId];
  return {
    draft: stored?.draft ?? { text: '', skills: [] },
    providerId: stored?.providerId ?? defaultProviderId,
  };
}

export function setSessionDraft(
  states: SessionInputStates,
  sessionId: string,
  draft: SessionInputDraft,
): SessionInputStates {
  return {
    ...states,
    [sessionId]: {
      ...states[sessionId],
      draft: {
        text: draft.text,
        skills: [...draft.skills],
      },
    },
  };
}

export function setSessionProvider(
  states: SessionInputStates,
  sessionId: string,
  providerId: string,
): SessionInputStates {
  return {
    ...states,
    [sessionId]: {
      ...states[sessionId],
      providerId,
    },
  };
}
