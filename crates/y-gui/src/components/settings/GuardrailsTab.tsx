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
import { Checkbox, Input } from '../ui';

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
        <div className="pf-row">
          <div className="pf-field">
            <label className="pf-label">Default Permission</label>
            <Select
              value={guardrailsForm.default_permission}
              onValueChange={(val) => { setGuardrailsForm({ ...guardrailsForm, default_permission: val }); setDirtyGuardrails(true); }}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select default permission" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="allow">Allow</SelectItem>
                <SelectItem value="notify">Notify</SelectItem>
                <SelectItem value="ask">Ask</SelectItem>
                <SelectItem value="deny">Deny</SelectItem>
              </SelectContent>
            </Select>
            <span className="pf-hint">Global default permission for tools.</span>
          </div>
          <div className="pf-field">
            <label className="pf-label">Max Tool Iterations</label>
            <Input
              numeric
              type="number"
              min={1}
              max={500}
              value={guardrailsForm.max_tool_iterations}
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, max_tool_iterations: Number(e.target.value) || 50 }); setDirtyGuardrails(true); }}
            />
            <span className="pf-hint">Max consecutive LLM calls in a tool-call loop.</span>
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <Checkbox
                checked={guardrailsForm.dangerous_auto_ask}
                onCheckedChange={(c) => { setGuardrailsForm({ ...guardrailsForm, dangerous_auto_ask: c === true }); setDirtyGuardrails(true); }}
              />
              {' '}Dangerous tools auto-escalate to "ask"
            </label>
          </div>
        </div>

        {/* Loop Guard */}
        <div className="pf-section-divider">
          <span className="pf-section-title">Loop Guard</span>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">Max Iterations</label>
            <Input
              numeric
              type="number"
              min={1}
              value={guardrailsForm.loop_guard_max_iterations}
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, loop_guard_max_iterations: Number(e.target.value) || 50 }); setDirtyGuardrails(true); }}
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Similarity Threshold</label>
            <Input
              numeric
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={guardrailsForm.loop_guard_similarity_threshold}
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, loop_guard_similarity_threshold: Number(e.target.value) || 0.95 }); setDirtyGuardrails(true); }}
            />
          </div>
        </div>

        {/* Risk Scoring */}
        <div className="pf-section-divider">
          <span className="pf-section-title">Risk Scoring</span>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">High Risk Threshold</label>
            <Input
              numeric
              type="number"
              min={0}
              max={1}
              step={0.1}
              value={guardrailsForm.risk_high_risk_threshold}
              onChange={(e) => { setGuardrailsForm({ ...guardrailsForm, risk_high_risk_threshold: Number(e.target.value) || 0.8 }); setDirtyGuardrails(true); }}
            />
          </div>
        </div>

        {/* HITL Escalation */}
        <div className="pf-section-divider">
          <span className="pf-section-title">HITL Escalation</span>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <Checkbox
                checked={guardrailsForm.hitl_auto_approve_low_risk}
                onCheckedChange={(c) => { setGuardrailsForm({ ...guardrailsForm, hitl_auto_approve_low_risk: c === true }); setDirtyGuardrails(true); }}
              />
              {' '}Auto-approve low-risk actions
            </label>
          </div>
        </div>
      </div>
    </>
  );
}
