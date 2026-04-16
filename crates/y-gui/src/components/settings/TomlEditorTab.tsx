// ---------------------------------------------------------------------------
// Raw TOML editor components for settings sections.
//
// Exports:
//   - RawTomlEditor: reusable Monaco editor for embedding in form tabs (RAW mode)
//   - RawModeToggle: switch toggle between Form and Raw TOML editing
// ---------------------------------------------------------------------------

import { MonacoEditor } from '../ui/MonacoEditor';

// ---------------------------------------------------------------------------
// RawTomlEditor -- Monaco editor, no loading/title management
// ---------------------------------------------------------------------------

interface RawTomlEditorProps {
  content: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function RawTomlEditor({ content, onChange, placeholder }: RawTomlEditorProps) {
  return (
    <div className="toml-editor-wrap">
      <MonacoEditor
        className="toml-editor-monaco"
        value={content}
        onChange={onChange}
        language="toml"
        placeholder={placeholder ?? 'No content. Will be created on save.'}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// RawModeToggle -- switch-style toggle between Form and Raw TOML editing
// ---------------------------------------------------------------------------

interface RawModeToggleProps {
  rawMode: boolean;
  onToggle: (next: boolean) => void;
}

export function RawModeToggle({ rawMode, onToggle }: RawModeToggleProps) {
  return (
    <label className="raw-mode-switch" title={rawMode ? 'Switch to Form view' : 'Switch to Raw TOML view'}>
      <span className={`raw-mode-switch-label ${rawMode ? '' : 'raw-mode-switch-label--active'}`}>Form</span>
      <button
        type="button"
        role="switch"
        aria-checked={rawMode}
        className={`raw-mode-switch-track ${rawMode ? 'raw-mode-switch-track--on' : ''}`}
        onClick={() => onToggle(!rawMode)}
      >
        <span className="raw-mode-switch-thumb" />
      </button>
      <span className={`raw-mode-switch-label ${rawMode ? 'raw-mode-switch-label--active' : ''}`}>RAW</span>
    </label>
  );
}
