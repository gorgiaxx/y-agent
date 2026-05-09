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

  it('keeps template actions in the left list and saves through Settings Save Changes', () => {
    const panel = readFileSync(
      new URL('../components/settings/SettingsPanel.tsx', import.meta.url),
      'utf8',
    );
    const tab = readFileSync(
      new URL('../components/settings/PromptTemplatesTab.tsx', import.meta.url),
      'utf8',
    );

    expect(panel).toContain('dirtyPromptTemplates');
    expect(panel).toContain('promptTemplateSaveHandler');
    expect(tab).toContain('sub-list-actions');
    expect(tab).toContain('sub-list-item-add');
    expect(tab).toContain('setDirtyPromptTemplates');
    expect(tab).toContain('makeDefault');
    expect(tab).toContain('Default prompt template cleared');
    expect(tab).not.toContain('Save Template');
    expect(tab).not.toContain('prompt-template-actions');
  });

  it('uses a split template editor with an independent adaptive preview panel', () => {
    const tab = readFileSync(
      new URL('../components/settings/PromptTemplatesTab.tsx', import.meta.url),
      'utf8',
    );
    const css = readFileSync(
      new URL('../components/settings/SettingsPanel.css', import.meta.url),
      'utf8',
    );

    expect(tab).toContain('prompt-template-layout');
    expect(tab).toContain('prompt-template-editor-column');
    expect(tab).toContain('showPreview={false}');
    expect(tab).toContain('PromptPreviewPanel');
    expect(tab).toContain('prompt-template-preview-panel');
    expect(tab).not.toContain('title="Description"');
    expect(tab).not.toContain('<Textarea');

    expect(css).toContain('.prompt-template-layout .sub-list-detail');
    expect(css).toContain('.prompt-template-detail');
    expect(css).toContain('grid-template-columns');
    expect(css).toContain('.prompt-template-preview-panel');
    expect(css).toContain('position: sticky');
    expect(css).toContain('calc(100vh');
  });
});
