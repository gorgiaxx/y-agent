// ---------------------------------------------------------------------------
// HooksTab -- Hook System (Middleware & Event Bus) configuration form
// ---------------------------------------------------------------------------

import type { HooksFormData } from './settingsTypes';
import { jsonToHooks } from './settingsTypes';
import { HOOKS_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
import { Input, SettingsGroup, SettingsItem } from '../ui';

interface HooksTabProps {
  loadSection: (section: string) => Promise<string>;
  hooksForm: HooksFormData;
  setHooksForm: React.Dispatch<React.SetStateAction<HooksFormData>>;
  setDirtyHooks: React.Dispatch<React.SetStateAction<boolean>>;
  setRawHooksToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function HooksTab({
  loadSection,
  hooksForm,
  setHooksForm,
  setDirtyHooks,
  setRawHooksToml,
}: HooksTabProps) {
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'hooks',
    schema: HOOKS_SCHEMA,
    configKey: 'hooks',
    form: hooksForm,
    setForm: setHooksForm,
    setDirty: setDirtyHooks,
    setRawToml: setRawHooksToml,
    jsonToForm: jsonToHooks,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No hooks.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="Hook System">
            <SettingsItem title="Middleware Timeout (ms)" description="Per-middleware timeout.">
              <Input
                numeric
                type="number"
                min={1000}
                step={1000}
                className="w-[100px]"
                value={hooksForm.middleware_timeout_ms}
                onChange={(e) => update({ middleware_timeout_ms: Number(e.target.value) || 30000 })}
              />
            </SettingsItem>
            <SettingsItem title="Event Channel Capacity" description="Channel capacity per subscriber.">
              <Input
                numeric
                type="number"
                min={64}
                step={256}
                className="w-[100px]"
                value={hooksForm.event_channel_capacity}
                onChange={(e) => update({ event_channel_capacity: Number(e.target.value) || 1024 })}
              />
            </SettingsItem>
            <SettingsItem title="Max Subscribers" description="Max event subscribers.">
              <Input
                numeric
                type="number"
                min={1}
                max={1024}
                className="w-[100px]"
                value={hooksForm.max_subscribers}
                onChange={(e) => update({ max_subscribers: Number(e.target.value) || 64 })}
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
