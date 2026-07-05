// ---------------------------------------------------------------------------
// StorageTab -- Storage (SQLite) configuration form
// ---------------------------------------------------------------------------

import type { StorageFormData } from './settingsTypes';
import { jsonToStorage } from './settingsTypes';
import { STORAGE_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
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
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'storage',
    schema: STORAGE_SCHEMA,
    configKey: 'storage',
    form: storageForm,
    setForm: setStorageForm,
    setDirty: setDirtyStorage,
    setRawToml: setRawStorageToml,
    jsonToForm: jsonToStorage,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No storage.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="Database">
            <SettingsItem
              title="Database Path"
              description={'Path to SQLite database file. Use ":memory:" for in-memory (testing).'}
              wide
            >
              <Input
                value={storageForm.db_path}
                onChange={(e) => update({ db_path: e.target.value })}
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
                onChange={(e) => update({ pool_size: Number(e.target.value) || 5 })}
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
                onChange={(e) => update({ busy_timeout_ms: Number(e.target.value) || 5000 })}
              />
            </SettingsItem>
            <SettingsItem
              title="Enable WAL Mode"
              description="Write-Ahead Logging recommended for concurrency."
            >
              <Checkbox
                checked={storageForm.wal_enabled}
                onCheckedChange={(c) => update({ wal_enabled: c === true })}
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
                onChange={(e) => update({ transcript_dir: e.target.value })}
                placeholder="data/transcripts"
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
