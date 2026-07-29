import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { KeyboardShortcutsTab } from '../components/settings/KeyboardShortcutsTab';
import { SettingsSidebarNav } from '../components/settings/SettingsSidebarNav';
import { defaultGuiConfig } from '../hooks/useConfig';

describe('keyboard shortcut settings', () => {
  it('registers the shortcut manager in shared desktop and web settings', () => {
    const sidebar = renderToStaticMarkup(
      <SettingsSidebarNav activeTab="keyboardShortcuts" onSelectTab={() => {}} />,
    );
    const tab = renderToStaticMarkup(
      <KeyboardShortcutsTab
        config={defaultGuiConfig}
        onChange={() => {}}
      />,
    );

    expect(sidebar).toContain('Keyboard Shortcuts');
    expect(tab).toContain('Search shortcuts');
    expect(tab).toContain('New chat');
    expect(tab).toContain('Open settings');
  });
});
