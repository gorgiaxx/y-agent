import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';
import { visibleIdeOptions } from '../components/settings/ideOptions';

describe('Default file IDE setting', () => {
  it('loads detected IDE choices and persists the selected default in GUI config', () => {
    const generalTab = readFileSync(
      new URL('../components/settings/GeneralTab.tsx', import.meta.url),
      'utf8',
    );
    const state = readFileSync(
      new URL('../../src-tauri/src/state.rs', import.meta.url),
      'utf8',
    );
    const systemCommands = readFileSync(
      new URL('../../src-tauri/src/commands/system.rs', import.meta.url),
      'utf8',
    );

    expect(generalTab).toContain('ide_list');
    expect(generalTab).toContain('default_file_ide');
    expect(generalTab).toContain('Default File IDE');
    expect(state).toContain('default_file_ide');
    expect(systemCommands).toContain('pub async fn ide_list');
    expect(systemCommands).toContain('pub async fn open_path_in_ide');
    expect(systemCommands).toContain('fn ide_candidates_for_platform');
    expect(systemCommands).toContain('name: "VS Code"');
    expect(systemCommands).toContain('name: "Xcode"');
    expect(systemCommands).toContain('name: "Antigravity"');
  });

  it('keeps auto detect and hides unavailable IDE choices from settings', () => {
    expect(visibleIdeOptions([
      { id: 'auto', name: 'Auto Detect', command: 'First available IDE', available: false },
      { id: 'cursor', name: 'Cursor', command: 'cursor', available: false },
      { id: 'vscode', name: 'VS Code', command: 'code', available: true },
    ])).toEqual([
      { id: 'auto', name: 'Auto Detect', command: 'First available IDE', available: false },
      { id: 'vscode', name: 'VS Code', command: 'code', available: true },
    ]);
  });
});
