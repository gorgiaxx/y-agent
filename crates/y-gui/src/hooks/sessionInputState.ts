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

interface PersistedSessionProviderSelections {
  version: 1;
  providers: Record<string, string>;
}

export function createSessionInputStates(): SessionInputStates {
  return {};
}

export function getSessionInputState(
  states: SessionInputStates,
  sessionId: string,
  defaultProviderId: string,
  availableProviderIds?: string[],
): ResolvedSessionInputState {
  const stored = states[sessionId];
  const candidateProviderId = stored?.providerId ?? defaultProviderId;
  const providerId = candidateProviderId === 'auto'
    || availableProviderIds === undefined
    || availableProviderIds.includes(candidateProviderId)
    ? candidateProviderId
    : 'auto';
  return {
    draft: stored?.draft ?? { text: '', skills: [] },
    providerId,
  };
}

export function serializeSessionProviderSelections(states: SessionInputStates): string {
  const providers = Object.fromEntries(
    Object.entries(states)
      .filter(([, state]) => typeof state.providerId === 'string' && state.providerId.trim())
      .map(([sessionId, state]) => [sessionId, state.providerId as string]),
  );
  const persisted: PersistedSessionProviderSelections = {
    version: 1,
    providers,
  };
  return JSON.stringify(persisted);
}

export function deserializeSessionProviderSelections(raw: string): SessionInputStates {
  try {
    const parsed = JSON.parse(raw) as Partial<PersistedSessionProviderSelections>;
    if (parsed.version !== 1 || !parsed.providers || typeof parsed.providers !== 'object') {
      return createSessionInputStates();
    }

    return Object.fromEntries(
      Object.entries(parsed.providers)
        .filter(([sessionId, providerId]) => (
          sessionId.length > 0
          && typeof providerId === 'string'
          && providerId.trim().length > 0
        ))
        .map(([sessionId, providerId]) => [sessionId, { providerId }]),
    );
  } catch {
    return createSessionInputStates();
  }
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
