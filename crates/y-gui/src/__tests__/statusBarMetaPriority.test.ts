import { describe, expect, it } from 'vitest';

import { shouldApplyMessageStatusFallback } from '../hooks/useStatusBarMeta';

describe('status bar metadata priority', () => {
  it('does not let persisted message metadata overwrite authoritative session metadata', () => {
    expect(shouldApplyMessageStatusFallback('session-1', 'session-1')).toBe(false);
    expect(shouldApplyMessageStatusFallback('session-1', null)).toBe(true);
  });
});
