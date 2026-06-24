// Pure reducer helpers for the per-session steering queue mirror.
//
// The backend is authoritative; these functions apply optimistic updates and
// reconcile against the `chat:steer_queue` broadcast. Kept side-effect free so
// they are trivially unit-testable (mirrors the chatRunState.ts pattern).

import type { SteerMessage } from '../types';

/** Per-session steering queues, keyed by session id. */
export type SteeringQueues = Record<string, SteerMessage[]>;

export function createSteeringQueues(): SteeringQueues {
  return {};
}

export function getQueue(state: SteeringQueues, sessionId: string): SteerMessage[] {
  return state[sessionId] ?? [];
}

/** Replace a session's queue with the authoritative server list. */
export function setQueue(
  state: SteeringQueues,
  sessionId: string,
  queue: SteerMessage[],
): SteeringQueues {
  if (queue.length === 0) {
    if (!(sessionId in state)) return state;
    const next = { ...state };
    delete next[sessionId];
    return next;
  }
  return { ...state, [sessionId]: queue };
}

/** Append a steer (idempotent by id) -- used for optimistic add. */
export function addSteer(
  state: SteeringQueues,
  sessionId: string,
  steer: SteerMessage,
): SteeringQueues {
  const existing = state[sessionId] ?? [];
  if (existing.some((s) => s.id === steer.id)) return state;
  return { ...state, [sessionId]: [...existing, steer] };
}

/** Remove a steer by id; drops the session key when the queue empties. */
export function removeSteer(
  state: SteeringQueues,
  sessionId: string,
  steerId: string,
): SteeringQueues {
  const existing = state[sessionId];
  if (!existing) return state;
  const filtered = existing.filter((s) => s.id !== steerId);
  if (filtered.length === existing.length) return state;
  return setQueue(state, sessionId, filtered);
}
