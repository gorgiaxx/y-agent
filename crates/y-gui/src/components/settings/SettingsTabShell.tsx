// ---------------------------------------------------------------------------
// SettingsTabShell -- shared shell for TOML-backed settings form tabs.
//
// Renders the loading guard, the raw-mode branch (action-slot toggle + raw
// TOML editor), or the form-mode branch (action-slot toggle + form body).
// Eliminates the loading guard, action-slot line, and raw-mode return block
// duplicated across the 9 canonical settings tabs.
// ---------------------------------------------------------------------------

import { type ReactNode } from 'react';
import { RawTomlEditor, RawModeToggle, SettingsActionSlot } from './TomlEditorTab';

export interface SettingsTabShellProps {
  loading: boolean;
  rawMode: boolean;
  rawContent: string;
  onToggleRaw: (next: boolean) => void;
  onRawChange: (val: string) => void;
  /** Placeholder shown in the raw TOML editor when the section file is absent. */
  rawPlaceholder: string;
  /** The form-mode body (rendered when not in raw mode). */
  form: ReactNode;
  /** Disable both form controls and RAW editing for an unavailable subsystem. */
  editingDisabled?: boolean;
}

export function SettingsTabShell({
  loading,
  rawMode,
  rawContent,
  onToggleRaw,
  onRawChange,
  rawPlaceholder,
  form,
  editingDisabled = false,
}: SettingsTabShellProps) {
  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <SettingsActionSlot>
          <RawModeToggle rawMode={rawMode} onToggle={onToggleRaw} disabled={editingDisabled} />
        </SettingsActionSlot>
        <RawTomlEditor
          content={rawContent}
          onChange={onRawChange}
          placeholder={rawPlaceholder}
          readOnly={editingDisabled}
        />
      </>
    );
  }

  return (
    <>
      <SettingsActionSlot>
        <RawModeToggle rawMode={rawMode} onToggle={onToggleRaw} disabled={editingDisabled} />
      </SettingsActionSlot>
      {form}
    </>
  );
}
