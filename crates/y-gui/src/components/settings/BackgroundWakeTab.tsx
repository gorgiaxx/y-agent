import type { RuntimeFeatureAvailability } from '../../types';
import { BACKGROUND_WAKE_SCHEMA } from '../../utils/settingsSchemas';
import { Checkbox, Input, SettingsGroup, SettingsItem, Switch } from '../ui';
import { FeatureAvailabilityNotice } from './FeatureAvailabilityNotice';
import type { BackgroundWakeFormData } from './settingsTypes';
import { jsonToBackgroundWake } from './settingsTypes';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';

interface BackgroundWakeFieldsProps {
  form: BackgroundWakeFormData;
  availability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
  onUpdate: (patch: Partial<BackgroundWakeFormData>) => void;
}

export function BackgroundWakeFields({
  form,
  availability,
  availabilityError,
  onUpdate,
}: BackgroundWakeFieldsProps) {
  const unavailable = !availability?.available;

  return (
    <div className="settings-form-wrap">
      <FeatureAvailabilityNotice
        featureName="Background auto-wake"
        availability={availability}
        error={availabilityError}
      />
      <fieldset disabled={unavailable} className="contents">
        <SettingsGroup
          title="Completion Wake Policy"
          description="Start a bounded automatic model turn when an eligible background task completes."
        >
          <SettingsItem title="Enable Automatic Wake">
            <Switch
              checked={form.enabled}
              onCheckedChange={(enabled) => onUpdate({ enabled })}
            />
          </SettingsItem>
          <SettingsItem title="Maximum Wakes per Hour" description="Rolling per-session budget for successfully started wake turns.">
            <Input
              numeric
              type="number"
              min={0}
              className="w-[100px]"
              value={form.max_wakes_per_hour}
              onChange={(event) => {
                onUpdate({ max_wakes_per_hour: Math.max(0, Number(event.target.value) || 0) });
              }}
            />
          </SettingsItem>
          <SettingsItem title="Cooldown (seconds)" description="Minimum spacing between successful wakes in one session.">
            <Input
              numeric
              type="number"
              min={0}
              className="w-[120px]"
              value={form.cooldown_secs}
              onChange={(event) => {
                onUpdate({ cooldown_secs: Math.max(0, Number(event.target.value) || 0) });
              }}
            />
          </SettingsItem>
          <SettingsItem
            title="Allow During Plan or Loop Execution"
            description="Disabled by default so background completions cannot interrupt active orchestration."
          >
            <Checkbox
              checked={form.allow_during_orchestration}
              onCheckedChange={(checked) => {
                onUpdate({ allow_during_orchestration: checked === true });
              }}
            />
          </SettingsItem>
        </SettingsGroup>
      </fieldset>
    </div>
  );
}

interface BackgroundWakeTabProps {
  loadSection: (section: string) => Promise<string>;
  form: BackgroundWakeFormData;
  setForm: React.Dispatch<React.SetStateAction<BackgroundWakeFormData>>;
  setDirty: React.Dispatch<React.SetStateAction<boolean>>;
  setRawToml: React.Dispatch<React.SetStateAction<string | undefined>>;
  availability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
}

export function BackgroundWakeTab({
  loadSection,
  form,
  setForm,
  setDirty,
  setRawToml,
  availability,
  availabilityError,
}: BackgroundWakeTabProps) {
  const settings = useSettingsTab({
    section: 'background_auto_wake',
    schema: BACKGROUND_WAKE_SCHEMA,
    configKey: 'background_auto_wake',
    form,
    setForm,
    setDirty,
    setRawToml,
    jsonToForm: jsonToBackgroundWake,
    loadSection,
  });
  const editingDisabled = !availability?.available;

  return (
    <SettingsTabShell
      loading={settings.loading}
      rawMode={settings.rawMode}
      rawContent={settings.rawContent}
      onToggleRaw={settings.handleToggleRaw}
      onRawChange={settings.handleRawChange}
      rawPlaceholder="No background_auto_wake.toml found. Content will be created on save."
      editingDisabled={editingDisabled}
      form={
        <BackgroundWakeFields
          form={form}
          availability={availability}
          availabilityError={availabilityError}
          onUpdate={settings.update}
        />
      }
    />
  );
}
