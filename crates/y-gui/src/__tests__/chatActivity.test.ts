import { describe, expect, it } from 'vitest';

import {
  CHAT_STUCK_TIMEOUT_MS,
  hasSessionActivityTimedOut,
} from '../hooks/chatActivity';

describe('chatActivity', () => {
  it('does not time out a session that is still receiving activity', () => {
    const now = 1_000_000;
    const lastActivityAt = now - (CHAT_STUCK_TIMEOUT_MS - 1);

    expect(hasSessionActivityTimedOut(lastActivityAt, now)).toBe(false);
  });

  it('times out a session only after it has been inactive for the full threshold', () => {
    const now = 1_000_000;
    const lastActivityAt = now - CHAT_STUCK_TIMEOUT_MS;

    expect(hasSessionActivityTimedOut(lastActivityAt, now)).toBe(true);
  });
});
