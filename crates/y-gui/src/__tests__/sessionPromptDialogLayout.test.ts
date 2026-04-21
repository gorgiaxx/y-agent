/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('SessionPromptDialog editor layout', () => {
  it('uses a dedicated fixed-height editor shell instead of the settings flex editor class', () => {
    const source = readFileSync(
      new URL('../components/chat-panel/SessionPromptDialog.tsx', import.meta.url),
      'utf8',
    );

    expect(source).toMatch(/className="session-prompt-editor"/);
    expect(source).toMatch(/className="session-prompt-editor__monaco"/);
    expect(source).not.toMatch(/className="prompt-editor-monaco"/);
  });

  it('keeps the editor shell from collapsing in a column flex dialog', () => {
    const css = readFileSync(
      new URL('../components/chat-panel/SessionPromptDialog.css', import.meta.url),
      'utf8',
    );

    expect(css).toMatch(
      /\.session-prompt-editor\s*\{[^}]*height:\s*280px;[^}]*flex:\s*0 0 280px;/s,
    );
    expect(css).toMatch(
      /\.session-prompt-editor__monaco\s*\{[^}]*height:\s*100%;[^}]*width:\s*100%;/s,
    );
  });
});
