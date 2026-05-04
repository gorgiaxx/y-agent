import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

function readSource(path: string): string {
  return readFileSync(new URL(path, import.meta.url), 'utf8');
}

describe('outside click handling', () => {
  it('centralizes dropdown outside-click listeners in a shared hook', () => {
    const inputArea = readSource('../components/chat-panel/input-area/InputArea.tsx');
    const hook = readSource('../hooks/useCloseOnOutsideClick.ts');

    expect(inputArea).not.toContain("document.addEventListener('mousedown'");
    expect(hook).toContain("document.addEventListener('mousedown'");
  });
});
