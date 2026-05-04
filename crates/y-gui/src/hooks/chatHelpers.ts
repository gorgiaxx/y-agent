// ---------------------------------------------------------------------------
// Chat helper utilities -- extracted from useChat.ts
//
//   - Per-session message cache (Map-based)
//   - Session lock for serialising compound operations
//   - Checkpoint resolution helpers
// ---------------------------------------------------------------------------

import { logger, transport } from '../lib';
import type { Message, ChatCheckpointInfo } from '../types';

// ---------------------------------------------------------------------------
// Per-session message cache
// ---------------------------------------------------------------------------

const MAX_CACHED_SESSIONS = 5;

export function getCachedMessages(
  cache: Map<string, Message[]>,
  sessionId: string,
): Message[] {
  return cache.get(sessionId) ?? [];
}

export function setCachedMessages(
  cache: Map<string, Message[]>,
  sessionId: string,
  updater: Message[] | ((prev: Message[]) => Message[]),
): Message[] {
  const prev = cache.get(sessionId) ?? [];
  const next = typeof updater === 'function' ? updater(prev) : updater;
  cache.set(sessionId, next);
  return next;
}

/**
 * Evict least-recently-used sessions from all shared ref Maps.
 *
 * Uses `sessionActivityRef` timestamps to determine staleness.
 * The active session and any streaming sessions are never evicted.
 */
export function evictStaleSessions(
  activeSessionId: string | null,
  streamingSessionIds: Set<string>,
  sessionActivityRef: Map<string, number>,
  ...caches: Map<string, unknown>[]
): void {
  const candidateIds = [...sessionActivityRef.keys()].filter(
    (id) => id !== activeSessionId && !streamingSessionIds.has(id),
  );

  if (candidateIds.length + (activeSessionId ? 1 : 0) <= MAX_CACHED_SESSIONS) {
    return;
  }

  candidateIds.sort(
    (a, b) => (sessionActivityRef.get(a) ?? 0) - (sessionActivityRef.get(b) ?? 0),
  );

  const evictCount =
    candidateIds.length + (activeSessionId ? 1 : 0) - MAX_CACHED_SESSIONS;
  const toEvict = candidateIds.slice(0, Math.max(0, evictCount));

  for (const id of toEvict) {
    for (const cache of caches) {
      cache.delete(id);
    }
    sessionActivityRef.delete(id);
  }

  if (toEvict.length > 0) {
    logger.debug(`[chat] evicted ${toEvict.length} stale session cache(s)`);
  }
}

/**
 * Remove all cached data for a specific session from every shared ref Map.
 *
 * Called when a session is deleted so its data does not linger in memory.
 */
export function purgeSessionCache(
  sessionId: string,
  ...caches: Map<string, unknown>[]
): void {
  for (const cache of caches) {
    cache.delete(sessionId);
  }
}

/**
 * Merge skill tag metadata from cached (optimistic) user messages into
 * backend-loaded messages. The backend doesn't persist `skills`, so we
 * transfer them from the cache by matching on role + content.
 */
export function mergeSkillsFromCache(
  backendMsgs: Message[],
  cache: Map<string, Message[]>,
  sessionId: string,
): Message[] {
  const cached = cache.get(sessionId);
  if (!cached || cached.length === 0) return backendMsgs;

  // Build a lookup: content -> skills (only for user messages with skills).
  const skillsByContent = new Map<string, string[]>();
  for (const m of cached) {
    if (m.role === 'user' && m.skills && m.skills.length > 0) {
      skillsByContent.set(m.content, m.skills);
    }
  }
  if (skillsByContent.size === 0) return backendMsgs;

  return backendMsgs.map((m) => {
    if (m.role === 'user' && !m.skills) {
      const skills = skillsByContent.get(m.content);
      if (skills) return { ...m, skills };
    }
    return m;
  });
}

// ---------------------------------------------------------------------------
// Session lock -- serialises compound operations per session
// ---------------------------------------------------------------------------

const sessionLocks = new Map<string, Promise<void>>();

export async function withSessionLock<T>(sessionId: string, fn: () => Promise<T>): Promise<T> {
  const prev = sessionLocks.get(sessionId) ?? Promise.resolve();
  let resolve: () => void;
  const next = new Promise<void>((r) => { resolve = r; });
  sessionLocks.set(sessionId, next);

  // Wait for previous operation to complete.
  await prev;

  try {
    return await fn();
  } finally {
    resolve!();
    if (sessionLocks.get(sessionId) === next) {
      sessionLocks.delete(sessionId);
    }
  }
}

// ---------------------------------------------------------------------------
// Checkpoint resolution
// ---------------------------------------------------------------------------

/** Find the checkpoint for a specific user message using the atomic backend
 *  command. Falls back to null if the message is not found or no checkpoint
 *  matches.
 */
export async function findCheckpointForMessage(
  sessionId: string,
  messageId: string,
  cache?: Map<string, Message[]>,
): Promise<ChatCheckpointInfo | null> {
  // Resolve content from cache so the backend can do content-based fallback.
  let content = '';
  if (cache) {
    const cachedMessages = cache.get(sessionId) ?? [];
    const cachedMsg = cachedMessages.find((m) => m.id === messageId);
    if (cachedMsg) {
      content = cachedMsg.content;
    }
  }

  try {
    return await transport.invoke<ChatCheckpointInfo | null>(
      'chat_find_checkpoint_for_resend',
      { sessionId, userMessageContent: content, messageId },
    );
  } catch (e) {
    logger.warn('[chat] findCheckpointForMessage: backend lookup failed:', e);
    return null;
  }
}
