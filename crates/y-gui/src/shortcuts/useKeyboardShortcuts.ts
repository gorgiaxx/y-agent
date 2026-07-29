import { useEffect, useMemo, useRef } from 'react';

import {
  findShortcutAction,
  isMacKeyboardPlatform,
  resolveShortcutBindings,
  type GuiShortcutActionId,
  type ShortcutBindings,
} from './shortcutRegistry';

export type ShortcutHandlers = Partial<Record<GuiShortcutActionId, () => void | Promise<void>>>;

export function dispatchKeyboardShortcut(
  event: KeyboardEvent,
  bindings: ShortcutBindings,
  handlers: ShortcutHandlers,
  isMac: boolean,
): boolean {
  if (event.defaultPrevented || event.isComposing || event.repeat) return false;
  const action = findShortcutAction(event, bindings, isMac);
  if (!action) return false;
  const handler = handlers[action];
  if (!handler) return false;

  event.preventDefault();
  event.stopPropagation();
  void handler();
  return true;
}

export function useKeyboardShortcuts(
  overrides: Record<string, string[]> | null | undefined,
  handlers: ShortcutHandlers,
): void {
  const bindings = useMemo(() => resolveShortcutBindings(overrides), [overrides]);
  const handlersRef = useRef(handlers);

  useEffect(() => {
    handlersRef.current = handlers;
  }, [handlers]);

  useEffect(() => {
    const isMac = isMacKeyboardPlatform();
    const handleKeyDown = (event: KeyboardEvent) => {
      dispatchKeyboardShortcut(event, bindings, handlersRef.current, isMac);
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [bindings]);
}
