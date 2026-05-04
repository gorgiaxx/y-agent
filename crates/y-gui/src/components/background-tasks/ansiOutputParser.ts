import type { CSSProperties } from 'react';

interface AnsiStyleState {
  fg?: string;
  bg?: string;
  color?: string;
  backgroundColor?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
}

export interface AnsiToken {
  text: string;
  className: string;
  style?: CSSProperties;
}

const ESCAPE = String.fromCharCode(27);
const BACKSPACE = String.fromCharCode(8);
const ANSI_SEQUENCE = new RegExp(`${ESCAPE}\\[[0-?]*[ -/]*[@-~]`, 'g');
const BACKSPACE_SEQUENCE = new RegExp(`[^\\n]${BACKSPACE}`, 'g');

function removeBackspaces(input: string): string {
  let current = input;
  let next = current.replace(BACKSPACE_SEQUENCE, '');
  while (next !== current) {
    current = next;
    next = current.replace(BACKSPACE_SEQUENCE, '');
  }
  return current;
}

export function normalizeTerminalText(input: string): string {
  return removeBackspaces(input)
    .split('\n')
    .map((line) => {
      const segments = line.split('\r');
      return segments[segments.length - 1] ?? '';
    })
    .join('\n');
}

function colorName(code: number): string | undefined {
  const names = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
  if (code >= 30 && code <= 37) return names[code - 30];
  if (code >= 90 && code <= 97) return `bright-${names[code - 90]}`;
  return undefined;
}

function bgColorName(code: number): string | undefined {
  const names = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
  if (code >= 40 && code <= 47) return names[code - 40];
  if (code >= 100 && code <= 107) return `bright-${names[code - 100]}`;
  return undefined;
}

function styleClasses(state: AnsiStyleState): string {
  const classes = ['ansi-token'];
  if (state.fg) classes.push(`ansi-fg-${state.fg}`);
  if (state.bg) classes.push(`ansi-bg-${state.bg}`);
  if (state.bold) classes.push('ansi-bold');
  if (state.dim) classes.push('ansi-dim');
  if (state.italic) classes.push('ansi-italic');
  if (state.underline) classes.push('ansi-underline');
  if (state.inverse) classes.push('ansi-inverse');
  return classes.join(' ');
}

function inlineStyle(state: AnsiStyleState): CSSProperties | undefined {
  if (!state.color && !state.backgroundColor) return undefined;
  return {
    color: state.color,
    backgroundColor: state.backgroundColor,
  };
}

function pushToken(tokens: AnsiToken[], text: string, state: AnsiStyleState) {
  if (!text) return;
  tokens.push({
    text,
    className: styleClasses(state),
    style: inlineStyle(state),
  });
}

function parseSgrCodes(sequence: string): number[] {
  const rawParams = sequence.slice(2, -1);
  if (!rawParams) return [0];
  return rawParams
    .split(';')
    .map((value) => Number.parseInt(value, 10))
    .map((value) => (Number.isFinite(value) ? value : 0));
}

function applySgr(sequence: string, state: AnsiStyleState) {
  const codes = parseSgrCodes(sequence);
  for (let i = 0; i < codes.length; i += 1) {
    const code = codes[i];
    if (code === 0) {
      state.fg = undefined;
      state.bg = undefined;
      state.color = undefined;
      state.backgroundColor = undefined;
      state.bold = false;
      state.dim = false;
      state.italic = false;
      state.underline = false;
      state.inverse = false;
    } else if (code === 1) {
      state.bold = true;
    } else if (code === 2) {
      state.dim = true;
    } else if (code === 3) {
      state.italic = true;
    } else if (code === 4) {
      state.underline = true;
    } else if (code === 7) {
      state.inverse = true;
    } else if (code === 22) {
      state.bold = false;
      state.dim = false;
    } else if (code === 23) {
      state.italic = false;
    } else if (code === 24) {
      state.underline = false;
    } else if (code === 27) {
      state.inverse = false;
    } else if (code === 39) {
      state.fg = undefined;
      state.color = undefined;
    } else if (code === 49) {
      state.bg = undefined;
      state.backgroundColor = undefined;
    } else if (code === 38 && codes[i + 1] === 2 && codes.length >= i + 5) {
      state.fg = undefined;
      state.color = `rgb(${codes[i + 2]}, ${codes[i + 3]}, ${codes[i + 4]})`;
      i += 4;
    } else if (code === 48 && codes[i + 1] === 2 && codes.length >= i + 5) {
      state.bg = undefined;
      state.backgroundColor = `rgb(${codes[i + 2]}, ${codes[i + 3]}, ${codes[i + 4]})`;
      i += 4;
    } else {
      const fg = colorName(code);
      const bg = bgColorName(code);
      if (fg) {
        state.fg = fg;
        state.color = undefined;
      }
      if (bg) {
        state.bg = bg;
        state.backgroundColor = undefined;
      }
    }
  }
}

export function tokenizeAnsi(input: string): AnsiToken[] {
  const normalized = normalizeTerminalText(input);
  const tokens: AnsiToken[] = [];
  const state: AnsiStyleState = {};
  let cursor = 0;

  for (const match of normalized.matchAll(ANSI_SEQUENCE)) {
    const index = match.index ?? 0;
    pushToken(tokens, normalized.slice(cursor, index), state);
    const sequence = match[0];
    if (sequence.endsWith('m')) {
      applySgr(sequence, state);
    }
    cursor = index + sequence.length;
  }

  pushToken(tokens, normalized.slice(cursor), state);
  return tokens;
}
