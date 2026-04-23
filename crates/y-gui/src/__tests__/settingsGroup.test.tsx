import { readFileSync } from 'node:fs';

import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { SettingsGroup } from '../components/ui/SettingsGroup';

describe('SettingsGroup', () => {
  it('renders a plain body variant for custom settings form bodies', () => {
    const html = renderToStaticMarkup(
      <SettingsGroup title="Custom" bodyVariant="plain">
        <div className="settings-item--custom-body">Custom editor</div>
      </SettingsGroup>,
    );

    expect(html).toContain('class="settings-group-body settings-group-body--plain"');
  });

  it('keeps the default bordered body variant when no style variant is provided', () => {
    const html = renderToStaticMarkup(
      <SettingsGroup title="Default">
        <div className="settings-item">Default editor</div>
      </SettingsGroup>,
    );

    expect(html).toContain('class="settings-group-body"');
    expect(html).not.toContain('settings-group-body--plain');
  });

  it('removes card chrome from the plain body variant in CSS', () => {
    const css = readFileSync(
      new URL('../components/settings/SettingsForm.css', import.meta.url),
      'utf8',
    );

    expect(css).toMatch(
      /\.settings-group-body--plain\s*\{[^}]*border:\s*0;[^}]*background:\s*transparent;/s,
    );
  });
});
