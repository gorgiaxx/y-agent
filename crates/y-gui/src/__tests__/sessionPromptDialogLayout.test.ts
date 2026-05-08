/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('SessionPromptDialog editor layout', () => {
  it('uses the shared prompt composer shell instead of the settings flex editor class', () => {
    const source = readFileSync(
      new URL('../components/chat-panel/SessionPromptDialog.tsx', import.meta.url),
      'utf8',
    );

    expect(source).toMatch(/className="session-prompt-composer"/);
    expect(source).toMatch(/<PromptComposer/);
    expect(source).not.toMatch(/className="prompt-editor-monaco"/);
  });

  it('keeps the composer editor shell from collapsing in a column flex dialog', () => {
    const dialogCss = readFileSync(
      new URL('../components/chat-panel/SessionPromptDialog.css', import.meta.url),
      'utf8',
    );
    const composerCss = readFileSync(
      new URL('../components/prompts/PromptComposer.css', import.meta.url),
      'utf8',
    );

    expect(dialogCss).toMatch(
      /\.session-prompt-composer\s*\{[^}]*max-height:\s*min\(62vh,\s*720px\);[^}]*overflow:\s*auto;/s,
    );
    expect(composerCss).toMatch(
      /\.prompt-composer-editor\s*\{[^}]*height:\s*260px;[^}]*min-height:\s*220px;/s,
    );
  });
});
