// ---------------------------------------------------------------------------
// GuardrailsTab -- Guardrails & Security configuration form
// ---------------------------------------------------------------------------

import type { GuardrailsFormData } from './settingsTypes';
import { jsonToGuardrails } from './settingsTypes';
import { GUARDRAILS_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

interface GuardrailsTabProps {
  loadSection: (section: string) => Promise<string>;
  guardrailsForm: GuardrailsFormData;
  setGuardrailsForm: React.Dispatch<React.SetStateAction<GuardrailsFormData>>;
  setDirtyGuardrails: React.Dispatch<React.SetStateAction<boolean>>;
  setRawGuardrailsToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function GuardrailsTab({
  loadSection,
  guardrailsForm,
  setGuardrailsForm,
  setDirtyGuardrails,
  setRawGuardrailsToml,
}: GuardrailsTabProps) {
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'guardrails',
    schema: GUARDRAILS_SCHEMA,
    configKey: 'guardrails',
    form: guardrailsForm,
    setForm: setGuardrailsForm,
    setDirty: setDirtyGuardrails,
    setRawToml: setRawGuardrailsToml,
    jsonToForm: jsonToGuardrails,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No guardrails.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="Permissions">
            <SettingsItem title="Default Permission" description="Global default permission for tools.">
              <Select
                value={guardrailsForm.default_permission}
                onValueChange={(val) => update({ default_permission: val })}
              >
                <SelectTrigger className="w-[140px]">
                  <SelectValue placeholder="Select default permission" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="allow">Allow</SelectItem>
                  <SelectItem value="notify">Notify</SelectItem>
                  <SelectItem value="ask">Ask</SelectItem>
                  <SelectItem value="deny">Deny</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
            <SettingsItem title="Plan Review Mode" description="Controls whether generated plans execute immediately or wait for chat review.">
              <Select
                value={guardrailsForm.plan_review_mode}
                onValueChange={(val) => update({ plan_review_mode: val as GuardrailsFormData['plan_review_mode'] })}
              >
                <SelectTrigger className="w-[160px]">
                  <SelectValue placeholder="Select plan review mode" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="auto">Auto</SelectItem>
                  <SelectItem value="manual">Manual review</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
            <SettingsItem title="Max Tool Iterations" description="Max consecutive LLM calls in a tool-call loop.">
              <Input
                numeric
                type="number"
                min={1}
                max={500}
                className="w-[100px]"
                value={guardrailsForm.max_tool_iterations}
                onChange={(e) => update({ max_tool_iterations: Number(e.target.value) || 50 })}
              />
            </SettingsItem>
            <SettingsItem title="Dangerous tools auto-escalate to ask">
              <Checkbox
                checked={guardrailsForm.dangerous_auto_ask}
                onCheckedChange={(c) => update({ dangerous_auto_ask: c === true })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="Loop Guard">
            <SettingsItem title="Max Iterations">
              <Input
                numeric
                type="number"
                min={1}
                className="w-[100px]"
                value={guardrailsForm.loop_guard_max_iterations}
                onChange={(e) => update({ loop_guard_max_iterations: Number(e.target.value) || 50 })}
              />
            </SettingsItem>
            <SettingsItem title="Similarity Threshold">
              <Input
                numeric
                type="number"
                min={0}
                max={1}
                step={0.05}
                className="w-[100px]"
                value={guardrailsForm.loop_guard_similarity_threshold}
                onChange={(e) => update({ loop_guard_similarity_threshold: Number(e.target.value) || 0.95 })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="Risk Scoring">
            <SettingsItem title="High Risk Threshold">
              <Input
                numeric
                type="number"
                min={0}
                max={1}
                step={0.1}
                className="w-[100px]"
                value={guardrailsForm.risk_high_risk_threshold}
                onChange={(e) => update({ risk_high_risk_threshold: Number(e.target.value) || 0.8 })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="HITL Escalation">
            <SettingsItem title="Auto-approve low-risk actions">
              <Checkbox
                checked={guardrailsForm.hitl_auto_approve_low_risk}
                onCheckedChange={(c) => update({ hitl_auto_approve_low_risk: c === true })}
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
