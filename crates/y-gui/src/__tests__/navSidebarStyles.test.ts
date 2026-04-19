/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('NavSidebar dark macOS styling', () => {
  it('overrides muted text color locally for the vibrancy sidebar', () => {
    const css = readFileSync(
      new URL('../components/common/NavSidebar/NavSidebar.css', import.meta.url),
      'utf8',
    );

    expect(css).toMatch(
      /html\[data-platform="macos"\]\[data-theme="dark"\]\s+\.nav-sidebar\s*\{[^}]*--text-muted:\s*#[0-9a-fA-F]{6};/s,
    );
  });
});
