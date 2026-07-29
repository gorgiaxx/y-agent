import { describe, expect, it } from 'vitest';

import {
  GUI_SHORTCUTS,
  findShortcutAction,
  findShortcutConflict,
  formatShortcutBinding,
  keyboardEventToBinding,
  resolveShortcutBindings,
} from '../shortcuts/shortcutRegistry';

function keyboardEvent(
  key: string,
  modifiers: Partial<Pick<KeyboardEvent, 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey'>> = {},
): KeyboardEvent {
  return {
    key,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    isComposing: false,
    repeat: false,
    defaultPrevented: false,
    ...modifiers,
  } as KeyboardEvent;
}

describe('GUI shortcut registry', () => {
  it('has stable unique action IDs', () => {
    const ids = GUI_SHORTCUTS.map((shortcut) => shortcut.id);

    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain('new_chat');
    expect(ids).toContain('keyboard_shortcuts');
  });

  it('merges action overrides while allowing an action to be unassigned', () => {
    const resolved = resolveShortcutBindings({
      new_chat: ['Mod+Shift+N'],
      open_settings: [],
      removed_action: ['Mod+R'],
    });

    expect(resolved.new_chat).toEqual(['Mod+Shift+N']);
    expect(resolved.open_settings).toEqual([]);
    expect(resolved.keyboard_shortcuts).toEqual(['Mod+K']);
    expect(resolved).not.toHaveProperty('removed_action');
  });

  it('normalizes a platform primary modifier to Mod', () => {
    expect(keyboardEventToBinding(keyboardEvent('n', { metaKey: true }), true)).toBe('Mod+N');
    expect(keyboardEventToBinding(keyboardEvent('n', { ctrlKey: true }), false)).toBe('Mod+N');
  });

  it('rejects modifier-only capture events', () => {
    expect(keyboardEventToBinding(keyboardEvent('Meta', { metaKey: true }), true)).toBeNull();
  });

  it('rejects unmodified printable keys for the global keymap', () => {
    expect(keyboardEventToBinding(keyboardEvent('n'), false)).toBeNull();
  });

  it('matches Mod bindings on macOS and non-macOS hosts', () => {
    const bindings = resolveShortcutBindings({ new_chat: ['Mod+N'] });

    expect(findShortcutAction(keyboardEvent('n', { metaKey: true }), bindings, true)).toBe('new_chat');
    expect(findShortcutAction(keyboardEvent('n', { ctrlKey: true }), bindings, false)).toBe('new_chat');
  });

  it('reports the existing action for a conflicting chord', () => {
    const bindings = resolveShortcutBindings({});

    expect(findShortcutConflict('Mod+N', 'open_settings', bindings)?.id).toBe('new_chat');
    expect(findShortcutConflict('Mod+,', 'open_settings', bindings)).toBeNull();
  });

  it('formats shortcut pills for the active platform', () => {
    expect(formatShortcutBinding('Mod+Shift+D', true)).toBe('\u21e7\u2318D');
    expect(formatShortcutBinding('Mod+Shift+D', false)).toBe('Ctrl+Shift+D');
  });
});
