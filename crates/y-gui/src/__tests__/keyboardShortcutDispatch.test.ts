import { describe, expect, it, vi } from 'vitest';

import { dispatchKeyboardShortcut } from '../shortcuts/useKeyboardShortcuts';
import { resolveShortcutBindings } from '../shortcuts/shortcutRegistry';

describe('GUI shortcut dispatcher', () => {
  it('dispatches exactly one semantic action and consumes the browser event', () => {
    const newChat = vi.fn();
    const openSettings = vi.fn();
    const preventDefault = vi.fn();
    const stopPropagation = vi.fn();
    const event = {
      key: 'n',
      altKey: false,
      ctrlKey: false,
      metaKey: true,
      shiftKey: false,
      isComposing: false,
      repeat: false,
      defaultPrevented: false,
      preventDefault,
      stopPropagation,
    } as unknown as KeyboardEvent;

    const handled = dispatchKeyboardShortcut(
      event,
      resolveShortcutBindings({ new_chat: ['Mod+N'], open_settings: ['Mod+,'] }),
      { new_chat: newChat, open_settings: openSettings },
      true,
    );

    expect(handled).toBe(true);
    expect(newChat).toHaveBeenCalledOnce();
    expect(openSettings).not.toHaveBeenCalled();
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(stopPropagation).toHaveBeenCalledOnce();
  });

  it('does not consume composing or repeated keyboard events', () => {
    const handler = vi.fn();
    const event = {
      key: 'n',
      altKey: false,
      ctrlKey: false,
      metaKey: true,
      shiftKey: false,
      isComposing: true,
      repeat: false,
      defaultPrevented: false,
    } as KeyboardEvent;

    expect(dispatchKeyboardShortcut(
      event,
      resolveShortcutBindings({ new_chat: ['Mod+N'] }),
      { new_chat: handler },
      true,
    )).toBe(false);
    expect(handler).not.toHaveBeenCalled();
  });
});
