import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('ChatSidebarPanel persistence', () => {
  it('uses the shared persistent-state hook for sort preferences', () => {
    const source = readFileSync(
      new URL('../components/chat-panel/ChatSidebarPanel.tsx', import.meta.url),
      'utf8',
    );

    expect(source).toContain('usePersistentState<SortField>');
    expect(source).not.toContain('localStorage.getItem(STORAGE_KEYS.WORKSPACE_SORT)');
    expect(source).not.toContain('localStorage.getItem(STORAGE_KEYS.SESSION_SORT)');
  });
});
