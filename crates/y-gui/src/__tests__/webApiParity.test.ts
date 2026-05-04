import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';
import { COMMAND_MAP } from '../lib/commandMap';

const WEB_LIFECYCLE_COMMANDS = new Set([
  'heartbeat_pong',
]);

const DESKTOP_ONLY_COMMANDS = new Set([
  'skill_open_folder',
]);

function readSource(path: string): string {
  return readFileSync(new URL(path, import.meta.url), 'utf8');
}

function registeredTauriCommands(): string[] {
  const source = readSource('../../src-tauri/src/lib.rs');
  const handlerBlock = source.match(/generate_handler!\[([\s\S]*?)\]\)/)?.[1] ?? '';
  return Array.from(handlerBlock.matchAll(/commands::[a-z_]+::([a-zA-Z0-9_]+)/g), (match) => match[1]);
}

describe('Web API and GUI command parity', () => {
  it('maps every shared Tauri command to HTTP or declares an explicit web behavior', () => {
    const mappedCommands = new Set(Object.keys(COMMAND_MAP));
    const missing = registeredTauriCommands().filter((command) => (
      !mappedCommands.has(command)
      && !WEB_LIFECYCLE_COMMANDS.has(command)
      && !DESKTOP_ONLY_COMMANDS.has(command)
    ));

    expect(missing).toEqual([]);
  });
});
