// ---------------------------------------------------------------------------
// MonacoEditorCore -- shared wrapper around react-monaco-editor
//
// Provides: offline worker setup, light/dark theme, Ctrl+S shortcut,
// sensible defaults for the y-gui desktop app.
// ---------------------------------------------------------------------------

import { useCallback, useRef, useEffect, type CSSProperties } from 'react';
import MonacoReactEditor from 'react-monaco-editor';
import type * as Monaco from 'monaco-editor';
import { useResolvedTheme } from '../../hooks/useTheme';

// ---------------------------------------------------------------------------
// Web worker setup (Vite ?worker imports -- bundled locally, no CDN)
// ---------------------------------------------------------------------------

import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker';
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker';
import cssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker';
import htmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker';
import tsWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker';

// Register Monarch tokenizers for all basic languages (Python, Markdown, Rust, etc.).
// Without this import the ESM build only ships tokenizers for worker-backed languages
// (JSON, CSS, HTML, TypeScript/JavaScript).
import 'monaco-editor/esm/vs/basic-languages/monaco.contribution';

interface MonacoEnvironmentGlobal extends Window {
  MonacoEnvironment?: {
    getWorker(_: unknown, label: string): Worker;
  };
}

const globalSelf = self as MonacoEnvironmentGlobal;

if (!globalSelf.MonacoEnvironment) {
  globalSelf.MonacoEnvironment = {
    getWorker(_: unknown, label: string) {
      if (label === 'json') return new jsonWorker();
      if (label === 'css' || label === 'scss' || label === 'less') return new cssWorker();
      if (label === 'html' || label === 'handlebars' || label === 'razor') return new htmlWorker();
      if (label === 'typescript' || label === 'javascript') return new tsWorker();
      return new editorWorker();
    },
  };
}

// ---------------------------------------------------------------------------
// TOML language registration (Monarch tokenizer)
// ---------------------------------------------------------------------------

let tomlRegistered = false;

function registerTomlLanguage(monacoInstance: typeof Monaco): void {
  if (tomlRegistered) return;
  tomlRegistered = true;

  monacoInstance.languages.register({ id: 'toml' });
  monacoInstance.languages.setMonarchTokensProvider('toml', {
    defaultToken: '',
    tokenPostfix: '.toml',

    escapes: /\\(?:[abfnrtv\\"']|x[0-9A-Fa-f]{1,4}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})/,

    tokenizer: {
      root: [
        // Comments
        [/#.*$/, 'comment'],

        // Table headers: [section], [[array_of_tables]]
        [/(\[\[)([^\]]+?)(\]\])/, ['delimiter.bracket', 'type', 'delimiter.bracket']],
        [/(\[)([^\]]+?)(\])/, ['delimiter.bracket', 'type', 'delimiter.bracket']],

        // Key = value
        [
          /^(\s*)([A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*)(\s*=\s*)/,
          ['', 'attribute.name', 'delimiter'],
        ],

        // Quoted keys
        [/^(\s*)(")(.*?)(")(\s*=\s*)/, ['', 'string', 'attribute.name', 'string', 'delimiter']],

        // Strings (multi-line basic, basic, literal, multi-line literal)
        [/"""/, 'string', '@multiline_basic_string'],
        [/'''/, 'string', '@multiline_literal_string'],
        [/"([^"\\]|\\.)*$/, 'string.invalid'],
        [/"/, 'string', '@basic_string'],
        [/'([^']*)'$/, 'string'],
        [/'/, 'string', '@literal_string'],

        // Booleans
        [/\b(true|false)\b/, 'keyword'],

        // Date/time (ISO 8601 variants)
        [
          /\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)?/,
          'number',
        ],

        // Numbers (float then int to avoid partial match)
        [/[+-]?\d+\.\d+(?:[eE][+-]?\d+)?/, 'number'],
        [/[+-]?\d+(?:_\d+)*(?:[eE][+-]?\d+)?/, 'number'],
        [/0x[0-9A-Fa-f]+/, 'number'],
        [/0o[0-7]+/, 'number'],
        [/0b[01]+/, 'number'],
        [/\binf\b/, 'number'],
        [/\bnan\b/, 'number'],
      ],

      basic_string: [
        [/[^\\"]+/, 'string'],
        [/@escapes/, 'string.escape'],
        [/\\./, 'string.escape.invalid'],
        [/"/, 'string', '@pop'],
      ],

      literal_string: [[/[^']+/, 'string'], [/'/, 'string', '@pop']],

      multiline_basic_string: [
        [/[^\\"]+/, 'string'],
        [/@escapes/, 'string.escape'],
        [/\\./, 'string.escape.invalid'],
        [/"""/, 'string', '@pop'],
        [/"/, 'string'],
      ],

      multiline_literal_string: [[/[^']+/, 'string'], [/'''/, 'string', '@pop'], [/'/, 'string']],
    },
  });

  // Auto-closing pairs for TOML
  monacoInstance.languages.setLanguageConfiguration('toml', {
    comments: {
      lineComment: '#',
    },
    brackets: [
      ['[', ']'],
      ['{', '}'],
    ],
    autoClosingPairs: [
      { open: '[', close: ']' },
      { open: '{', close: '}' },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    surroundingPairs: [
      { open: '[', close: ']' },
      { open: '{', close: '}' },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    folding: {
      markers: {
        start: /^\s*\[\[/,
        end: /^\s*\[\[/,
      },
    },
  });
}

// ---------------------------------------------------------------------------
// Theme definitions
// ---------------------------------------------------------------------------

const DARK_THEME = 'y-gui-dark';
const LIGHT_THEME = 'y-gui-light';

let themesRegistered = false;

function registerThemes(monacoInstance: typeof Monaco): void {
  if (themesRegistered) return;
  themesRegistered = true;

  monacoInstance.editor.defineTheme(DARK_THEME, {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: '', foreground: 'd4d4d4', background: '1e1e1e' },
      { token: 'comment', foreground: '6a9955' },
      { token: 'keyword', foreground: '569cd6' },
      { token: 'string', foreground: 'ce9178' },
      { token: 'string.escape', foreground: 'd7ba7d' },
      { token: 'number', foreground: 'b5cea8' },
      { token: 'type', foreground: '4ec9b0' },
      { token: 'delimiter', foreground: 'd4d4d4' },
      { token: 'delimiter.bracket', foreground: 'ffd700' },
      { token: 'tag', foreground: '569cd6' },
      { token: 'attribute.name', foreground: '9cdcfe' },
      { token: 'attribute.value', foreground: 'ce9178' },
    ],
    colors: {
      'editor.background': '#1a1a1a',
      'editor.foreground': '#d4d4d4',
      'editor.lineHighlightBackground': '#2a2a2a',
      'editor.selectionBackground': '#264f78',
      'editorCursor.foreground': '#aeafad',
      'editor.inactiveSelectionBackground': '#3a3d4110',
      'editorLineNumber.foreground': '#858585',
      'editorLineNumber.activeForeground': '#c6c6c6',
      'editor.selectionHighlightBackground': '#add6ff26',
      'editorIndentGuide.background': '#404040',
      'editorIndentGuide.activeBackground': '#707070',
      'editorBracketMatch.background': '#0064001a',
      'editorBracketMatch.border': '#888888',
    },
  });

  monacoInstance.editor.defineTheme(LIGHT_THEME, {
    base: 'vs',
    inherit: true,
    rules: [
      { token: '', foreground: '1a1917', background: 'ffffff' },
      { token: 'comment', foreground: '6a9955' },
      { token: 'keyword', foreground: '0000ff' },
      { token: 'string', foreground: 'a31515' },
      { token: 'string.escape', foreground: 'e50000' },
      { token: 'number', foreground: '098658' },
      { token: 'type', foreground: '267f99' },
      { token: 'delimiter', foreground: '1a1917' },
      { token: 'delimiter.bracket', foreground: '811f3f' },
      { token: 'tag', foreground: '800000' },
      { token: 'attribute.name', foreground: 'e50000' },
      { token: 'attribute.value', foreground: 'a31515' },
    ],
    colors: {
      'editor.background': '#ffffff',
      'editor.foreground': '#1a1917',
      'editor.lineHighlightBackground': '#f5f4f1',
      'editor.selectionBackground': '#add6ff',
      'editorCursor.foreground': '#1a1917',
      'editor.inactiveSelectionBackground': '#add6ff40',
      'editorLineNumber.foreground': '#9c9894',
      'editorLineNumber.activeForeground': '#6b6560',
      'editor.selectionHighlightBackground': '#add6ff40',
      'editorIndentGuide.background': '#edecea',
      'editorIndentGuide.activeBackground': '#9c9894',
      'editorBracketMatch.background': '#0064001a',
      'editorBracketMatch.border': '#b0b0b0',
    },
  });
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface MonacoEditorProps {
  /** Controlled value */
  value: string;
  /** Called when content changes */
  onChange?: (value: string) => void;
  /** Language identifier (e.g. 'toml', 'markdown', 'plaintext') */
  language?: string;
  /** Editor height -- CSS value. When omitted, the container CSS controls sizing (e.g. flex). */
  height?: string | number;
  /** Editor width -- CSS value. When omitted, the container CSS controls sizing. */
  width?: string | number;
  /** Additional CSS class on the wrapper div */
  className?: string;
  /** Ctrl+S / Cmd+S handler -- registered via Monaco addCommand */
  onSave?: () => void;
  /** Read-only mode */
  readOnly?: boolean;
  /** Placeholder text shown when value is empty */
  placeholder?: string;
  /** Additional Monaco editor options (merged over defaults) */
  options?: Monaco.editor.IStandaloneEditorConstructionOptions;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function MonacoEditorCore({
  value,
  onChange,
  language = 'plaintext',
  height,
  width,
  className,
  onSave,
  readOnly = false,
  placeholder,
  options = {},
}: MonacoEditorProps) {
  const resolvedTheme = useResolvedTheme();
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<typeof Monaco | null>(null);

  const monacoTheme = resolvedTheme === 'dark' ? DARK_THEME : LIGHT_THEME;

  const handleEditorDidMount = useCallback(
    (editor: Monaco.editor.IStandaloneCodeEditor, monacoInstance: typeof Monaco) => {
      editorRef.current = editor;
      monacoRef.current = monacoInstance;

      registerThemes(monacoInstance);
      registerTomlLanguage(monacoInstance);
      monacoInstance.editor.setTheme(resolvedTheme === 'dark' ? DARK_THEME : LIGHT_THEME);

      if (onSave) {
        editor.addCommand(
          monacoInstance.KeyMod.CtrlCmd | monacoInstance.KeyCode.KeyS,
          () => onSave(),
        );
      }

      editor.focus();
    },
    [onSave, resolvedTheme],
  );

  // Switch Monaco theme when the resolved theme changes.
  useEffect(() => {
    const monacoInstance = monacoRef.current;
    if (!monacoInstance) return;
    registerThemes(monacoInstance);
    monacoInstance.editor.setTheme(monacoTheme);
  }, [monacoTheme]);

  // When onSave changes, re-register the command.
  useEffect(() => {
    const editor = editorRef.current;
    const monacoInstance = monacoRef.current;
    if (!editor || !monacoInstance || !onSave) return;

    // addCommand on the same keybinding replaces the previous one.
    editor.addCommand(
      monacoInstance.KeyMod.CtrlCmd | monacoInstance.KeyCode.KeyS,
      () => onSave(),
    );
  }, [onSave]);

  const handleChange = useCallback(
    (newValue: string) => {
      onChange?.(newValue);
    },
    [onChange],
  );

  const defaultOptions: Monaco.editor.IStandaloneEditorConstructionOptions = {
    minimap: { enabled: false },
    lineNumbers: 'on',
    wordWrap: 'on',
    tabSize: 2,
    fontSize: 13,
    scrollBeyondLastLine: false,
    readOnly,
    renderLineHighlight: 'line',
    scrollbar: {
      verticalScrollbarSize: 8,
      horizontalScrollbarSize: 8,
    },
    padding: { top: 8, bottom: 8 },
    overviewRulerBorder: false,
    hideCursorInOverviewRuler: true,
    renderWhitespace: 'none',
    guides: {
      indentation: true,
    },
    quickSuggestions: false,
    suggestOnTriggerCharacters: false,
    parameterHints: { enabled: false },
    autoClosingBrackets: 'never',
    autoClosingQuotes: 'never',
    folding: true,
    lineDecorationsWidth: 4,
    lineNumbersMinChars: 3,
  };

  const mergedOptions: Monaco.editor.IStandaloneEditorConstructionOptions = {
    ...defaultOptions,
    ...options,
    readOnly,
  };

  const displayValue = value || placeholder || '';

  const wrapperStyle: CSSProperties = {};
  if (height != null) wrapperStyle.height = height;
  if (width != null) wrapperStyle.width = width;

  return (
    <div className={className} style={wrapperStyle}>
      <MonacoReactEditor
        height="100%"
        width="100%"
        language={language}
        theme={monacoTheme}
        value={displayValue}
        onChange={handleChange}
        options={mergedOptions}
        editorDidMount={handleEditorDidMount}
      />
    </div>
  );
}

MonacoEditorCore.displayName = 'MonacoEditorCore';
