// GUI command definitions and filter logic.
//
// Mirrors the command vocabulary from the TUI CommandRegistry
// (crates/y-cli/src/tui/commands/registry.rs) but scoped to
// Phase 1 GUI-relevant commands.

export type CommandCategory = 'Session' | 'General' | 'Debug' | 'Model';

export interface GuiCommandDef {
  name: string;
  alias: string | null;
  description: string;
  category: CommandCategory;
  /** If true, the command executes immediately on selection (no arguments needed). */
  immediate: boolean;
}

export const GUI_COMMANDS: GuiCommandDef[] = [
  // Session commands
  { name: 'new', alias: 'n', description: 'Create new session', category: 'Session', immediate: true },
  { name: 'export', alias: null, description: 'Export session', category: 'Session', immediate: true },
  { name: 'compact', alias: null, description: 'Compact context (prune + summarize)', category: 'Session', immediate: true },
  { name: 'rewind', alias: null, description: 'Rewind to a previous message (undo file changes)', category: 'Session', immediate: true },
  // General commands
  { name: 'clear', alias: 'cl', description: 'Clear current session', category: 'General', immediate: true },
  { name: 'help', alias: 'h', description: 'Show help', category: 'General', immediate: true },
  { name: 'settings', alias: null, description: 'Open settings', category: 'General', immediate: true },
  // Debug commands
  { name: 'status', alias: 'st', description: 'Show system status', category: 'Debug', immediate: true },
  { name: 'diagnostics', alias: null, description: 'Toggle diagnostics panel', category: 'Debug', immediate: true },
  { name: 'observability', alias: null, description: 'Toggle observability panel', category: 'Debug', immediate: true },
  // Model commands
  { name: 'model', alias: 'm', description: 'Model selection', category: 'Model', immediate: false },
];

/** Category display order for grouped rendering. */
export const CATEGORY_ORDER: CommandCategory[] = ['Session', 'General', 'Debug', 'Model'];

/**
 * Filter commands by query string using prefix matching on name, alias,
 * and substring matching on description (case-insensitive).
 *
 * Matches the same logic as Rust `CommandRegistry::search`.
 */
export function filterCommands(query: string): GuiCommandDef[] {
  if (!query) return GUI_COMMANDS;
  const q = query.toLowerCase();
  return GUI_COMMANDS.filter(
    (cmd) =>
      cmd.name.startsWith(q) ||
      (cmd.alias !== null && cmd.alias.startsWith(q)) ||
      cmd.description.toLowerCase().includes(q),
  );
}
