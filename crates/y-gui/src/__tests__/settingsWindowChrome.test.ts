/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('Settings window chrome', () => {
  it('marks the settings action bar as a Tauri drag region', () => {
    const source = readFileSync(
      new URL('../components/settings/SettingsPanel.tsx', import.meta.url),
      'utf8',
    );

    expect(source).toMatch(
      /<div className="settings-action-bar" data-tauri-drag-region>/,
    );
  });

  it('keeps settings action controls out of the drag hit area in custom decorations mode', () => {
    const css = readFileSync(
      new URL('../components/settings/SettingsPanel.css', import.meta.url),
      'utf8',
    );

    expect(css).toMatch(
      /html\.custom-decorations\s+\.settings-action-bar\s*\{[^}]*-webkit-app-region:\s*drag;/s,
    );
    expect(css).toMatch(
      /html\.custom-decorations\s+\.settings-action-bar-actions\s*\{[^}]*-webkit-app-region:\s*no-drag;/s,
    );
  });
});
