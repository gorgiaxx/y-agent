// ---------------------------------------------------------------------------
// GuardrailsTab -- Guardrails & Security configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { GuardrailsFormData } from './settingsTypes';
import { jsonToGuardrails } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { GUARDRAILS_SCHEMA } from '../../utils/settingsSchemas';
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
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const json = allConfig?.guardrails ?? {};
      setGuardrailsForm(jsonToGuardrails(json));
      try {
        const raw = await loadSection('guardrails');
        setRawGuardrailsToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawGuardrailsToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setGuardrailsForm, setRawGuardrailsToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, guardrailsForm as unknown as Record<string, unknown>, GUARDRAILS_SCHEMA));
    }
    setRawMode(next);
  }, [guardrailsForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle">Guardrails <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawGuardrailsToml(val);
            setDirtyGuardrails(true);
          }}
          placeholder="No guardrails.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle">Guardrails <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Permissions">
          <SettingsItem title="Default Permission" description="Global default permission for tools.">
            <Select
              value={guardrailsForm.default_permission}
              onValueChange={(val) => { setGuardrailsForm({ ...guardrailsForm, default_permission: val }); setDirtyGuardrails(true); }}
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
          <SettingsItem title="Max Tool Iterations" description="Max consecutive LLM calls in a tool-call loop.">
            <Input
              numeric
              type="number"
              min={1}
              max={500}
              className="w-[100px]"
              value={guardrailsForm.max_tool_iterations}
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, max_tool_iterations: Number(e.target.value) || 50 }); setDirtyGuardrails(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Dangerous tools auto-escalate to ask">
            <Checkbox
              checked={guardrailsForm.dangerous_auto_ask}
              onCheckedChange={(c) => { setGuardrailsForm({ ...guardrailsForm, dangerous_auto_ask: c === true }); setDirtyGuardrails(true); }}
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
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, loop_guard_max_iterations: Number(e.target.value) || 50 }); setDirtyGuardrails(true); }}
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
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, loop_guard_similarity_threshold: Number(e.target.value) || 0.95 }); setDirtyGuardrails(true); }}
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
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, risk_high_risk_threshold: Number(e.target.value) || 0.8 }); setDirtyGuardrails(true); }}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="HITL Escalation">
          <SettingsItem title="Auto-approve low-risk actions">
            <Checkbox
              checked={guardrailsForm.hitl_auto_approve_low_risk}
              onCheckedChange={(c) => { setGuardrailsForm({ ...guardrailsForm, hitl_auto_approve_low_risk: c === true }); setDirtyGuardrails(true); }}
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
