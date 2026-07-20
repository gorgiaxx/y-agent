// ---------------------------------------------------------------------------
// Raw TOML editor components for settings sections.
//
// Exports:
//   - RawTomlEditor: reusable Monaco editor for embedding in form tabs (RAW mode)
//   - RawModeToggle: switch toggle between Form and Raw TOML editing
//   - SettingsActionSlotProvider / SettingsActionSlotTarget / SettingsActionSlot:
//     portal plumbing that lets the active tab render its Form/RAW toggle into
//     the shared settings action bar (left of Save Changes) instead of inline.
// ---------------------------------------------------------------------------

import { createContext, useContext, useMemo, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { MonacoEditor } from '../ui/MonacoEditor';
import { Switch } from '../ui/Switch';

// ---------------------------------------------------------------------------
// RawTomlEditor -- Monaco editor, no loading/title management
// ---------------------------------------------------------------------------

interface RawTomlEditorProps {
  content: string;
  onChange: (value: string) => void;
  placeholder?: string;
  readOnly?: boolean;
}

export function RawTomlEditor({ content, onChange, placeholder, readOnly = false }: RawTomlEditorProps) {
  return (
    <div className="toml-editor-wrap">
      <MonacoEditor
        className="toml-editor-monaco"
        value={content}
        onChange={onChange}
        language="toml"
        readOnly={readOnly}
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
  disabled?: boolean;
}

export function RawModeToggle({ rawMode, onToggle, disabled = false }: RawModeToggleProps) {
  return (
    <label className="raw-mode-switch" title={rawMode ? 'Switch to Form view' : 'Switch to Raw TOML view'}>
      <span className={`raw-mode-switch-label ${rawMode ? '' : 'raw-mode-switch-label--active'}`}>Form</span>
      <Switch
        checked={rawMode}
        onCheckedChange={onToggle}
        disabled={disabled}
      />
      <span className={`raw-mode-switch-label ${rawMode ? 'raw-mode-switch-label--active' : ''}`}>RAW</span>
    </label>
  );
}

// ---------------------------------------------------------------------------
// SettingsActionSlot -- portal plumbing for the shared action-bar toggle.
//
// Only one settings tab is mounted at a time, so the active tab can portal its
// Form/RAW toggle into a single shared target rendered in the action bar.
// ---------------------------------------------------------------------------

interface SettingsActionSlotValue {
  target: HTMLElement | null;
  setTarget: (el: HTMLElement | null) => void;
}

const SettingsActionSlotContext = createContext<SettingsActionSlotValue | null>(null);

export function SettingsActionSlotProvider({ children }: { children: ReactNode }) {
  const [target, setTarget] = useState<HTMLElement | null>(null);
  const value = useMemo(() => ({ target, setTarget }), [target]);
  return (
    <SettingsActionSlotContext.Provider value={value}>
      {children}
    </SettingsActionSlotContext.Provider>
  );
}

// Placed inside the action bar; hosts the portal target the active tab's
// toggle renders into.
export function SettingsActionSlotTarget({ className }: { className?: string }) {
  const ctx = useContext(SettingsActionSlotContext);
  return <div className={className} ref={ctx?.setTarget} />;
}

// Renders its children into the shared action-bar slot via portal.
export function SettingsActionSlot({ children }: { children: ReactNode }) {
  const ctx = useContext(SettingsActionSlotContext);
  if (!ctx?.target) return null;
  return createPortal(children, ctx.target);
}
