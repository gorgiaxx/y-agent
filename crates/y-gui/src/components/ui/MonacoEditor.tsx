// ---------------------------------------------------------------------------
// MonacoEditor -- shared wrapper around react-monaco-editor
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

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const globalSelf = self as any;

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
      { token: 'number', foreground: 'b5cea8' },
      { token: 'type', foreground: '4ec9b0' },
      { token: 'delimiter', foreground: 'd4d4d4' },
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
      { token: 'number', foreground: '098658' },
      { token: 'type', foreground: '267f99' },
      { token: 'delimiter', foreground: '1a1917' },
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

export function MonacoEditor({
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

MonacoEditor.displayName = 'MonacoEditor';
