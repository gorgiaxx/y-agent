export type GuiShortcutActionId =
  | 'new_chat'
  | 'focus_composer'
  | 'open_settings'
  | 'keyboard_shortcuts'
  | 'show_chat'
  | 'show_automation'
  | 'show_skills'
  | 'show_knowledge'
  | 'show_agents'
  | 'toggle_diagnostics'
  | 'toggle_observability'
  | 'toggle_info';

export interface GuiShortcutDefinition {
  id: GuiShortcutActionId;
  title: string;
  description: string;
  category: 'General' | 'Navigation' | 'Panels';
  defaultBindings: readonly string[];
}

export type ShortcutBindings = Record<GuiShortcutActionId, string[]>;

export const GUI_SHORTCUTS: readonly GuiShortcutDefinition[] = [
  {
    id: 'new_chat',
    title: 'New chat',
    description: 'Start a new chat in the current workspace',
    category: 'General',
    defaultBindings: ['Mod+N'],
  },
  {
    id: 'focus_composer',
    title: 'Focus composer',
    description: 'Move focus to the chat composer',
    category: 'General',
    defaultBindings: ['Mod+Shift+L'],
  },
  {
    id: 'open_settings',
    title: 'Open settings',
    description: 'Open application settings',
    category: 'General',
    defaultBindings: ['Mod+,'],
  },
  {
    id: 'keyboard_shortcuts',
    title: 'Keyboard shortcuts',
    description: 'Open the keyboard shortcut manager',
    category: 'General',
    defaultBindings: ['Mod+K'],
  },
  {
    id: 'show_chat',
    title: 'Show chat',
    description: 'Switch to the chat workspace',
    category: 'Navigation',
    defaultBindings: ['Mod+1'],
  },
  {
    id: 'show_automation',
    title: 'Show automation',
    description: 'Switch to automation',
    category: 'Navigation',
    defaultBindings: ['Mod+2'],
  },
  {
    id: 'show_skills',
    title: 'Show skills',
    description: 'Switch to skills',
    category: 'Navigation',
    defaultBindings: ['Mod+3'],
  },
  {
    id: 'show_knowledge',
    title: 'Show knowledge',
    description: 'Switch to the knowledge base',
    category: 'Navigation',
    defaultBindings: ['Mod+4'],
  },
  {
    id: 'show_agents',
    title: 'Show agents',
    description: 'Switch to agents',
    category: 'Navigation',
    defaultBindings: ['Mod+5'],
  },
  {
    id: 'toggle_diagnostics',
    title: 'Toggle diagnostics',
    description: 'Show or hide the diagnostics panel',
    category: 'Panels',
    defaultBindings: ['Mod+Shift+D'],
  },
  {
    id: 'toggle_observability',
    title: 'Toggle observability',
    description: 'Show or hide the observability panel',
    category: 'Panels',
    defaultBindings: ['Mod+Shift+O'],
  },
  {
    id: 'toggle_info',
    title: 'Toggle info',
    description: 'Show or hide the session info panel',
    category: 'Panels',
    defaultBindings: ['Mod+Shift+I'],
  },
];

const DEFINITION_BY_ID = new Map(GUI_SHORTCUTS.map((shortcut) => [shortcut.id, shortcut]));
const MODIFIER_KEYS = new Set(['Alt', 'Control', 'Meta', 'Shift']);
const MODIFIER_ORDER = ['Mod', 'Ctrl', 'Alt', 'Shift', 'Meta'] as const;

function normalizeKey(key: string): string | null {
  if (MODIFIER_KEYS.has(key)) return null;
  if (key === ' ') return 'Space';
  if (key === 'Escape') return 'Esc';
  if (key === 'ArrowUp') return 'Up';
  if (key === 'ArrowDown') return 'Down';
  if (key === 'ArrowLeft') return 'Left';
  if (key === 'ArrowRight') return 'Right';
  if (key.length === 1) return key.toUpperCase();
  return key;
}

export function normalizeShortcutBinding(binding: string): string | null {
  const parts = binding.split('+').map((part) => part.trim()).filter(Boolean);
  const rawKey = parts.pop();
  if (!rawKey) return null;

  const modifiers = new Set<string>();
  for (const part of parts) {
    const normalized = part.toLowerCase();
    if (normalized === 'mod' || normalized === 'cmd' || normalized === 'command') {
      modifiers.add('Mod');
    } else if (normalized === 'ctrl' || normalized === 'control') {
      modifiers.add('Ctrl');
    } else if (normalized === 'alt' || normalized === 'option') {
      modifiers.add('Alt');
    } else if (normalized === 'shift') {
      modifiers.add('Shift');
    } else if (normalized === 'meta' || normalized === 'super') {
      modifiers.add('Meta');
    } else {
      return null;
    }
  }

  const key = normalizeKey(rawKey);
  if (!key) return null;
  return [...MODIFIER_ORDER.filter((modifier) => modifiers.has(modifier)), key].join('+');
}

export function keyboardEventToBinding(event: KeyboardEvent, isMac: boolean): string | null {
  const key = normalizeKey(event.key);
  if (!key) return null;

  const modifiers: string[] = [];
  if ((isMac && event.metaKey) || (!isMac && event.ctrlKey)) modifiers.push('Mod');
  if (isMac && event.ctrlKey) modifiers.push('Ctrl');
  if (event.altKey) modifiers.push('Alt');
  if (event.shiftKey) modifiers.push('Shift');
  if (!isMac && event.metaKey) modifiers.push('Meta');

  const isFunctionKey = /^F(?:[1-9]|1[0-9]|2[0-4])$/.test(key);
  if (modifiers.length === 0 && !isFunctionKey) return null;
  return [...modifiers, key].join('+');
}

export function resolveShortcutBindings(
  overrides: Record<string, string[]> | null | undefined,
): ShortcutBindings {
  const resolved = {} as ShortcutBindings;
  for (const shortcut of GUI_SHORTCUTS) {
    const configured = Object.prototype.hasOwnProperty.call(overrides ?? {}, shortcut.id)
      ? overrides?.[shortcut.id] ?? []
      : shortcut.defaultBindings;
    resolved[shortcut.id] = Array.from(new Set(
      configured
        .map(normalizeShortcutBinding)
        .filter((binding): binding is string => binding !== null),
    ));
  }
  return resolved;
}

export function findShortcutAction(
  event: KeyboardEvent,
  bindings: ShortcutBindings,
  isMac: boolean,
): GuiShortcutActionId | null {
  const binding = keyboardEventToBinding(event, isMac);
  if (!binding) return null;
  return GUI_SHORTCUTS.find((shortcut) => bindings[shortcut.id].includes(binding))?.id ?? null;
}

export function findShortcutConflict(
  binding: string,
  actionId: GuiShortcutActionId,
  bindings: ShortcutBindings,
): GuiShortcutDefinition | null {
  const normalized = normalizeShortcutBinding(binding);
  if (!normalized) return null;
  return GUI_SHORTCUTS.find(
    (shortcut) => shortcut.id !== actionId && bindings[shortcut.id].includes(normalized),
  ) ?? null;
}

export function formatShortcutBinding(binding: string, isMac: boolean): string {
  const normalized = normalizeShortcutBinding(binding) ?? binding;
  const parts = normalized.split('+');
  const key = parts.pop() ?? '';
  if (!isMac) {
    return [...parts.map((part) => part === 'Mod' ? 'Ctrl' : part), key].join('+');
  }

  const symbols: Record<string, string> = {
    Shift: '\u21e7',
    Ctrl: '\u2303',
    Alt: '\u2325',
    Mod: '\u2318',
    Meta: '\u25c6',
  };
  const order = ['Shift', 'Ctrl', 'Alt', 'Mod', 'Meta'];
  return `${order.filter((part) => parts.includes(part)).map((part) => symbols[part]).join('')}${key}`;
}

export function getShortcutDefinition(id: GuiShortcutActionId): GuiShortcutDefinition {
  const definition = DEFINITION_BY_ID.get(id);
  if (!definition) throw new Error(`Unknown GUI shortcut action: ${id}`);
  return definition;
}

export function isMacKeyboardPlatform(): boolean {
  return typeof navigator !== 'undefined' && /mac|iphone|ipad/i.test(navigator.platform);
}
