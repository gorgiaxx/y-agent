/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('Session prompt editor surface', () => {
  it('opens session prompt editing as a main panel instead of a dialog', () => {
    const inputArea = readFileSync(
      new URL('../components/chat-panel/input-area/InputArea.tsx', import.meta.url),
      'utf8',
    );
    const panel = readFileSync(
      new URL('../components/chat-panel/SessionPromptPanel.tsx', import.meta.url),
      'utf8',
    );
    const layout = readFileSync(
      new URL('../layouts/MainLayout.tsx', import.meta.url),
      'utf8',
    );

    expect(inputArea).toContain('onEditSessionPrompt');
    expect(inputArea).not.toContain('SessionPromptDialog');
    expect(panel).toContain('settings-action-bar');
    expect(panel).toContain('Save Changes');
    expect(panel).not.toContain('<Dialog');
    expect(panel).not.toContain('DialogContent');
    expect(layout).toContain('<SessionPromptPanel');
  });

  it('uses a back-only sidebar while editing a session prompt', () => {
    const sidebar = readFileSync(
      new URL('../components/Sidebar.tsx', import.meta.url),
      'utf8',
    );

    expect(sidebar).toContain('sessionPromptEditing');
    expect(sidebar).toContain('onSessionPromptBack');
    expect(sidebar).toContain('Session prompt editor');
    expect(sidebar).toContain('label="Back"');
  });
});
