import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { SettingsSidebarNav } from '../components/settings/SettingsSidebarNav';
import { TAB_LABELS } from '../components/settings/settingsTypes';

describe('optional capability settings navigation', () => {
  it('exposes background wake and LSP in the shared desktop/web sidebar', () => {
    const html = renderToStaticMarkup(
      <SettingsSidebarNav activeTab="backgroundWake" onSelectTab={() => {}} />,
    );

    expect(html).toContain('Background Wake');
    expect(html).toContain('Language Servers');
    expect(html).toContain('Capability Packs');
    expect(TAB_LABELS.backgroundWake).toBe('Background Wake');
    expect(TAB_LABELS.lsp).toBe('Language Servers');
    expect(TAB_LABELS.capabilityPacks).toBe('Capability Packs');
  });
});
