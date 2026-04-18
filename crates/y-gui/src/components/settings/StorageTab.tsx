// ---------------------------------------------------------------------------
// StorageTab -- Storage (SQLite) configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { StorageFormData } from './settingsTypes';
import { jsonToStorage } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { STORAGE_SCHEMA } from '../../utils/settingsSchemas';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

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
      const allConfig = await transport.invoke<any>('config_get');
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
          <h3 className="section-title section-title--flush">
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
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle">Storage <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Database">
          <SettingsItem
            title="Database Path"
            description={'Path to SQLite database file. Use ":memory:" for in-memory (testing).'}
            wide
          >
            <Input
              value={storageForm.db_path}
              onChange={(e) => { setStorageForm({ ...storageForm, db_path: e.target.value }); setDirtyStorage(true); }}
              placeholder="data/y-agent.db"
            />
          </SettingsItem>
          <SettingsItem title="Pool Size">
            <Input
              numeric
              type="number"
              min={1}
              max={100}
              className="w-[100px]"
              value={storageForm.pool_size}
              onChange={(e) => { setStorageForm({ ...storageForm, pool_size: Number(e.target.value) || 5 }); setDirtyStorage(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Busy Timeout (ms)">
            <Input
              numeric
              type="number"
              min={100}
              step={500}
              className="w-[100px]"
              value={storageForm.busy_timeout_ms}
              onChange={(e) => { setStorageForm({ ...storageForm, busy_timeout_ms: Number(e.target.value) || 5000 }); setDirtyStorage(true); }}
            />
          </SettingsItem>
          <SettingsItem
            title="Enable WAL Mode"
            description="Write-Ahead Logging recommended for concurrency."
          >
            <Checkbox
              checked={storageForm.wal_enabled}
              onCheckedChange={(c) => { setStorageForm({ ...storageForm, wal_enabled: c === true }); setDirtyStorage(true); }}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Transcripts">
          <SettingsItem
            title="Transcript Directory"
            description="Directory for JSONL session transcripts."
            wide
          >
            <Input
              value={storageForm.transcript_dir}
              onChange={(e) => { setStorageForm({ ...storageForm, transcript_dir: e.target.value }); setDirtyStorage(true); }}
              placeholder="data/transcripts"
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
