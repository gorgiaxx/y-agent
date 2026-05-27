import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import { getVisiblePendingEdit } from '../hooks/chatEditState';
import type { PendingEdit } from '../hooks/useChat';

describe('chat edit state', () => {
  it('shows a pending edit only for the session where it was created', () => {
    const edit: PendingEdit = {
      messageId: 'message-1',
      content: 'revise this',
      sessionId: 'session-a',
    };

    expect(getVisiblePendingEdit(edit, 'session-a')).toBe(edit);
    expect(getVisiblePendingEdit(edit, 'session-b')).toBeNull();
  });

  it('keeps legacy pending edit objects visible when no session id is present', () => {
    const edit: PendingEdit = {
      messageId: 'message-1',
      content: 'revise this',
    };

    expect(getVisiblePendingEdit(edit, 'session-a')).toBe(edit);
  });

  it('clears pending edit before edit resend starts async undo work', () => {
    const source = readFileSync(
      new URL('../hooks/useChatOperations.ts', import.meta.url),
      'utf8',
    );
    const editAndResendStart = source.indexOf('const editAndResend = useCallback');
    const lockStart = source.indexOf('return withSessionLock', editAndResendStart);
    const clearStart = source.indexOf('setPendingEdit(null);', editAndResendStart);

    expect(editAndResendStart).toBeGreaterThanOrEqual(0);
    expect(lockStart).toBeGreaterThan(editAndResendStart);
    expect(clearStart).toBeGreaterThan(editAndResendStart);
    expect(clearStart).toBeLessThan(lockStart);
  });
});
