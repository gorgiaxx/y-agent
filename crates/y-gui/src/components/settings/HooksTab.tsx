// ---------------------------------------------------------------------------
// HooksTab -- Hook System (Middleware & Event Bus) configuration form
// ---------------------------------------------------------------------------

import type { HooksFormData } from './settingsTypes';
import type { RuntimeFeatureAvailability } from '../../types';
import { jsonToHooks } from './settingsTypes';
import { HOOKS_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
import { Input, SettingsGroup, SettingsItem } from '../ui';
import { Checkbox } from '../ui';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/Select';
import { FeatureAvailabilityNotice } from './FeatureAvailabilityNotice';
import { TagChipInput } from './TagChipInput';

interface HooksAdvancedFieldsProps {
  form: HooksFormData;
  handlerAvailability: RuntimeFeatureAvailability | null | undefined;
  llmHookAvailability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
  onUpdate: (patch: Partial<HooksFormData>) => void;
}

export function HooksAdvancedFields({
  form,
  handlerAvailability,
  llmHookAvailability,
  availabilityError,
  onUpdate,
}: HooksAdvancedFieldsProps) {
  const handlersUnavailable = !handlerAvailability?.available;

  return (
    <SettingsGroup
      title="External Hook Handlers"
      description="Configuration-driven command, HTTP, prompt, and agent handlers."
    >
      <FeatureAvailabilityNotice
        featureName="Hook handlers"
        availability={handlerAvailability}
        error={availabilityError}
        plural
      />
      {availabilityError ? null : !llmHookAvailability ? (
        <FeatureAvailabilityNotice
          featureName="LLM hooks"
          availability={llmHookAvailability}
        />
      ) : !llmHookAvailability.available ? (
        <div className="rounded-md border border-solid border-[var(--border)] bg-[var(--surface-secondary)] p-3 text-11px text-[var(--text-secondary)]">
          Prompt and agent handlers require LLM hooks, which are not compiled into this binary. Command and HTTP handler declarations remain separate.
        </div>
      ) : null}
      <fieldset disabled={handlersUnavailable} className="contents">
        <SettingsItem title="Enable External Handlers">
          <Checkbox
            checked={form.handlers_enabled}
            onCheckedChange={(checked) => onUpdate({ handlers_enabled: checked === true })}
          />
        </SettingsItem>
        <SettingsItem
          title="Allowed Hook Directories"
          description="Command scripts must use absolute paths inside these directories. Empty allows any absolute directory."
          wide
        >
          <TagChipInput
            tags={form.allowed_hook_dirs}
            onChange={(allowed_hook_dirs) => onUpdate({ allowed_hook_dirs })}
          />
        </SettingsItem>
        <SettingsItem title="Payload Verbosity" description="Full includes raw content and should be enabled only when required.">
          <Select
            value={form.verbosity}
            onValueChange={(verbosity) => {
              onUpdate({ verbosity: verbosity as HooksFormData['verbosity'] });
            }}
          >
            <SelectTrigger className="w-[180px]">
              <SelectValue placeholder="Select verbosity" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="minimal">Minimal</SelectItem>
              <SelectItem value="standard">Standard</SelectItem>
              <SelectItem value="full">Full raw content</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
      </fieldset>
      <div className="text-11px text-[var(--text-muted)]">
        Handler groups remain editable in RAW TOML mode so matchers and typed handler payloads preserve the canonical configuration shape.
      </div>
    </SettingsGroup>
  );
}

interface HooksTabProps {
  loadSection: (section: string) => Promise<string>;
  hooksForm: HooksFormData;
  setHooksForm: React.Dispatch<React.SetStateAction<HooksFormData>>;
  setDirtyHooks: React.Dispatch<React.SetStateAction<boolean>>;
  setRawHooksToml: React.Dispatch<React.SetStateAction<string | undefined>>;
  handlerAvailability?: RuntimeFeatureAvailability | null;
  llmHookAvailability?: RuntimeFeatureAvailability | null;
  availabilityError?: string | null;
}

export function HooksTab({
  loadSection,
  hooksForm,
  setHooksForm,
  setDirtyHooks,
  setRawHooksToml,
  handlerAvailability,
  llmHookAvailability,
  availabilityError,
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
          <HooksAdvancedFields
            form={hooksForm}
            handlerAvailability={handlerAvailability}
            llmHookAvailability={llmHookAvailability}
            availabilityError={availabilityError}
            onUpdate={update}
          />
        </div>
      }
    />
  );
}
