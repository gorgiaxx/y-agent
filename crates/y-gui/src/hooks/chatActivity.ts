export const CHAT_STUCK_TIMEOUT_MS = 5 * 60 * 1000;

export function hasSessionActivityTimedOut(
  lastActivityAt: number | undefined,
  now: number = Date.now(),
  timeoutMs: number = CHAT_STUCK_TIMEOUT_MS,
): boolean {
  if (lastActivityAt == null) {
    return false;
  }

  return now - lastActivityAt >= timeoutMs;
}
