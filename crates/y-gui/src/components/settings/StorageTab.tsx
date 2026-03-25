// ---------------------------------------------------------------------------
// StorageTab -- Storage (SQLite) configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { StorageFormData } from './settingsTypes';
import { jsonToStorage } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { STORAGE_SCHEMA } from '../../utils/settingsSchemas';
import { Checkbox } from '../ui';

interface StorageTabProps {
  loadSection: (section: string) => Promise<string>;
  storageForm: StorageFormData;
  setStorageForm: React.Dispatch<React.SetStateAction<StorageFormData>>;
  setDirtyStorage: React.Dispatch<React.SetStateAction<boolean>>;
  setRawStorageToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function StorageTab({
  loadSection,
  storageForm,
  setStorageForm,
  setDirtyStorage,
  setRawStorageToml,
}: StorageTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const json = allConfig?.storage ?? {};
      setStorageForm(jsonToStorage(json));
      try {
        const raw = await loadSection('storage');
        setRawStorageToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawStorageToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setStorageForm, setRawStorageToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      // Form -> Raw: merge form data into cached raw TOML to preserve comments.
      setRawContent(mergeIntoRawToml(cachedRawToml.current, storageForm as unknown as Record<string, unknown>, STORAGE_SCHEMA));
    }
    setRawMode(next);
  }, [storageForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
            <span className="settings-header-with-toggle">Storage <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawStorageToml(val);
            setDirtyStorage(true);
          }}
          placeholder="No storage.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
          <span className="settings-header-with-toggle">Storage <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">Database Path</label>
            <input
              className="pf-input"
              value={storageForm.db_path}
              onChange={(e) => { setStorageForm({ ...storageForm, db_path: e.target.value }); setDirtyStorage(true); }}
              placeholder="data/y-agent.db"
            />
            <span className="pf-hint">Path to SQLite database file. Use ":memory:" for in-memory (testing).</span>
          </div>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">Pool Size</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={1}
              max={100}
              value={storageForm.pool_size}
              onChange={(e) => { setStorageForm({ ...storageForm, pool_size: Number(e.target.value) || 5 }); setDirtyStorage(true); }}
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Busy Timeout (ms)</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={100}
              step={500}
              value={storageForm.busy_timeout_ms}
              onChange={(e) => { setStorageForm({ ...storageForm, busy_timeout_ms: Number(e.target.value) || 5000 }); setDirtyStorage(true); }}
            />
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <Checkbox
                checked={storageForm.wal_enabled}
                onCheckedChange={(c) => { setStorageForm({ ...storageForm, wal_enabled: c === true }); setDirtyStorage(true); }}
              />
              {' '}Enable WAL Mode
            </label>
            <span className="pf-hint">Write-Ahead Logging recommended for concurrency.</span>
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">Transcript Directory</label>
            <input
              className="pf-input"
              value={storageForm.transcript_dir}
              onChange={(e) => { setStorageForm({ ...storageForm, transcript_dir: e.target.value }); setDirtyStorage(true); }}
              placeholder="data/transcripts"
            />
            <span className="pf-hint">Directory for JSONL session transcripts.</span>
          </div>
        </div>
      </div>
    </>
  );
}
