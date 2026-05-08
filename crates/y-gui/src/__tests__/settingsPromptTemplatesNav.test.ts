import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('Prompt template settings navigation', () => {
  it('exposes Prompt Templates as a separate top-level settings menu from Builtin Prompts', () => {
    const nav = readFileSync(
      new URL('../components/settings/SettingsSidebarNav.tsx', import.meta.url),
      'utf8',
    );
    const panel = readFileSync(
      new URL('../components/settings/SettingsPanel.tsx', import.meta.url),
      'utf8',
    );
    const types = readFileSync(
      new URL('../components/settings/settingsTypes.ts', import.meta.url),
      'utf8',
    );

    expect(nav).toContain("key: 'promptTemplates', label: 'Prompt Templates'");
    expect(nav).toContain("key: 'prompts', label: 'Builtin Prompts'");
    expect(panel).toContain("TabsContent value=\"promptTemplates\"");
    expect(types).toContain("promptTemplates: 'Prompt Templates'");
  });
});
