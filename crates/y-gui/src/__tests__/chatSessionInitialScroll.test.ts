import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('chat session initial scroll', () => {
  it('remounts the panel per session and initializes Virtuoso at the bottom', () => {
    const chatView = readFileSync(new URL('../views/ChatView.tsx', import.meta.url), 'utf8');
    const chatPanel = readFileSync(
      new URL('../components/chat-panel/ChatPanel.tsx', import.meta.url),
      'utf8',
    );

    expect(chatView).toContain('key={sessionHooks.activeSessionId}');
    expect(chatPanel).toContain('initialTopMostItemIndex={resolveInitialTopMostItemIndex(');
  });
});
