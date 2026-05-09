import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('Langfuse settings tab navigation', () => {
  it('registers the langfuse tab in sidebar nav, panel, and type labels', () => {
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

    expect(nav).toContain("key: 'langfuse', label: 'Langfuse'");
    expect(nav).toContain('langfuse:');
    expect(panel).toContain('TabsContent value="langfuse"');
    expect(panel).toContain('dirtyLangfuse');
    expect(panel).toContain('langfuseForm');
    expect(panel).toContain('rawLangfuseToml');
    expect(types).toContain("langfuse: 'Langfuse'");
  });

  it('places langfuse between knowledge and promptTemplates in the nav order', () => {
    const nav = readFileSync(
      new URL('../components/settings/SettingsSidebarNav.tsx', import.meta.url),
      'utf8',
    );
    const knowledgeIdx = nav.indexOf("key: 'knowledge'");
    const langfuseIdx = nav.indexOf("key: 'langfuse'");
    const promptTemplatesIdx = nav.indexOf("key: 'promptTemplates'");

    expect(knowledgeIdx).toBeLessThan(langfuseIdx);
    expect(langfuseIdx).toBeLessThan(promptTemplatesIdx);
  });
});
