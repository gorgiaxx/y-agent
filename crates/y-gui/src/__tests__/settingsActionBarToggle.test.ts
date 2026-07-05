/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

const read = (rel: string) =>
  readFileSync(new URL(rel, import.meta.url), 'utf8');
// Tabs that expose a Form/RAW toggle and must render it into the shared
// action-bar slot rather than an inline section header.
//
// SHELL_TABS use the shared SettingsTabShell (canonical form tabs).
// INLINE_TABS have a list+detail layout and render the action-slot inline.
const SHELL_TABS = [
  'SessionTab',
  'RuntimeTab',
  'BrowserTab',
  'StorageTab',
  'HooksTab',
  'ToolsTab',
  'GuardrailsTab',
  'KnowledgeTab',
  'LangfuseTab',
] as const;

const INLINE_TABS = [
  'ProvidersTab',
  'McpTab',
] as const;

describe('Settings action-bar Form/RAW toggle slot', () => {
  it('exposes a portal slot helper from TomlEditorTab', () => {
    const src = read('../components/settings/TomlEditorTab.tsx');
    expect(src).toContain("import { createPortal } from 'react-dom'");
    expect(src).toContain('export function SettingsActionSlotProvider');
    expect(src).toContain('export function SettingsActionSlot');
  });

  it('renders the toggle slot to the left of Save Changes and provides it to the tabs', () => {
    const src = read('../components/settings/SettingsPanel.tsx');
    expect(src).toContain('settings-action-bar-toggle-slot');
    expect(src).toContain('SettingsActionSlotProvider');

    const slotIdx = src.indexOf('settings-action-bar-toggle-slot');
    const saveIdx = src.indexOf("'Save Changes'");
    expect(slotIdx).toBeGreaterThan(-1);
    expect(saveIdx).toBeGreaterThan(-1);
    // The slot must appear before the Save Changes button in the action bar.
    expect(slotIdx).toBeLessThan(saveIdx);

    // The provider must wrap the tab content.
    const providerIdx = src.indexOf('<SettingsActionSlotProvider');
    const tabsIdx = src.indexOf('<Tabs ');
    expect(providerIdx).toBeGreaterThan(-1);
    expect(providerIdx).toBeLessThan(tabsIdx);
  });

  it('styles the slot without adding a phantom gap when empty', () => {
    const css = read('../components/settings/SettingsPanel.css');
    expect(css).toMatch(
      /\.settings-action-bar-toggle-slot\s*\{[^}]*display:\s*contents;/s,
    );
  });

  it.each(SHELL_TABS)('%s portals its toggle into the action-bar slot via SettingsTabShell', (tab) => {
    const src = read(`../components/settings/${tab}.tsx`);
    // Shell tabs render the shell, which owns the action-slot + toggle.
    expect(src).toContain('SettingsTabShell');
    // The old inline header-with-toggle wrapper must be gone.
    expect(src).not.toContain('settings-header-with-toggle');
  });

  it.each(INLINE_TABS)('%s portals its toggle into the action-bar slot inline', (tab) => {
    const src = read(`../components/settings/${tab}.tsx`);
    expect(src).toContain('SettingsActionSlot');
    expect(src).not.toContain('settings-header-with-toggle');
  });

  it('drops the now-dead inline header-with-toggle rule', () => {
    const css = read('../components/settings/ProvidersTab.css');
    expect(css).not.toContain('.settings-header-with-toggle');
  });
});
