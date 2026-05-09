// ---------------------------------------------------------------------------
// LangfuseTab -- Langfuse OTLP export configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { transport } from '../../lib';
import type { LangfuseFormData } from './settingsTypes';
import { jsonToLangfuse } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { LANGFUSE_SCHEMA } from '../../utils/settingsSchemas';
import { TagChipInput } from './TagChipInput';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

interface LangfuseTabProps {
  loadSection: (section: string) => Promise<string>;
  langfuseForm: LangfuseFormData;
  setLangfuseForm: React.Dispatch<React.SetStateAction<LangfuseFormData>>;
  setDirtyLangfuse: React.Dispatch<React.SetStateAction<boolean>>;
  setRawLangfuseToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function LangfuseTab({
  loadSection,
  langfuseForm,
  setLangfuseForm,
  setDirtyLangfuse,
  setRawLangfuseToml,
}: LangfuseTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);
  const [showPublicKey, setShowPublicKey] = useState(false);
  const [showSecretKey, setShowSecretKey] = useState(false);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await transport.invoke<any>('config_get');
      const json = allConfig?.langfuse ?? {};
      setLangfuseForm(jsonToLangfuse(json));
      try {
        const raw = await loadSection('langfuse');
        setRawLangfuseToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawLangfuseToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setLangfuseForm, setRawLangfuseToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, langfuseForm as unknown as Record<string, unknown>, LANGFUSE_SCHEMA));
    }
    setRawMode(next);
  }, [langfuseForm]);

  const update = useCallback((patch: Partial<LangfuseFormData>) => {
    setLangfuseForm((prev) => ({ ...prev, ...patch }));
    setDirtyLangfuse(true);
  }, [setLangfuseForm, setDirtyLangfuse]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawLangfuseToml(val);
            setDirtyLangfuse(true);
          }}
          placeholder="No langfuse.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Connection">
          <SettingsItem title="Enabled" description="Enable Langfuse OTLP trace export.">
            <Checkbox
              checked={langfuseForm.enabled}
              onCheckedChange={(c) => update({ enabled: c === true })}
            />
          </SettingsItem>
          <SettingsItem title="" description="">
            <span className="settings-notice">Changes require an application restart to take effect.</span>
          </SettingsItem>
          <SettingsItem title="Base URL" description="Langfuse endpoint (cloud or self-hosted).">
            <Input
              className="w-full"
              value={langfuseForm.base_url}
              placeholder="https://cloud.langfuse.com"
              onChange={(e) => update({ base_url: e.target.value })}
            />
          </SettingsItem>
          <SettingsItem title="Public Key">
            <div className="pf-key-group w-full">
              <Input
                className="w-full"
                type={showPublicKey ? 'text' : 'password'}
                value={langfuseForm.public_key}
                placeholder="pk-lf-..."
                onChange={(e) => update({ public_key: e.target.value })}
              />
              <button
                type="button"
                className="pf-key-toggle"
                onClick={() => setShowPublicKey(!showPublicKey)}
              >
                {showPublicKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </SettingsItem>
          <SettingsItem title="Secret Key">
            <div className="pf-key-group w-full">
              <Input
                className="w-full"
                type={showSecretKey ? 'text' : 'password'}
                value={langfuseForm.secret_key}
                placeholder="sk-lf-..."
                onChange={(e) => update({ secret_key: e.target.value })}
              />
              <button
                type="button"
                className="pf-key-toggle"
                onClick={() => setShowSecretKey(!showSecretKey)}
              >
                {showSecretKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </SettingsItem>
          <SettingsItem title="Project ID" description="Optional. Auto-detected from keys.">
            <Input
              className="w-full"
              value={langfuseForm.project_id}
              placeholder="(optional)"
              onChange={(e) => update({ project_id: e.target.value })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Content Capture">
          <SettingsItem title="Capture Input" description="Export prompt messages as GenAI events.">
            <Checkbox
              checked={langfuseForm.content_capture_input}
              onCheckedChange={(c) => update({ content_capture_input: c === true })}
            />
          </SettingsItem>
          <SettingsItem title="Capture Output" description="Export responses as GenAI choice events.">
            <Checkbox
              checked={langfuseForm.content_capture_output}
              onCheckedChange={(c) => update({ content_capture_output: c === true })}
            />
          </SettingsItem>
          <SettingsItem title="Max Content Length" description="Truncate content fields at this character limit.">
            <Input
              numeric
              type="number"
              min={0}
              className="w-[120px]"
              value={langfuseForm.content_max_content_length}
              onChange={(e) => update({ content_max_content_length: Number(e.target.value) || 10000 })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Sampling">
          <SettingsItem title="Sampling Rate" description="Fraction of traces to export (0.0 = none, 1.0 = all).">
            <Input
              numeric
              type="number"
              min={0}
              max={1}
              step={0.1}
              className="w-[100px]"
              value={langfuseForm.sampling_rate}
              onChange={(e) => update({ sampling_rate: Number(e.target.value) || 1.0 })}
            />
          </SettingsItem>
          <SettingsItem title="Include Tags" description="Only export traces with these tags.">
            <TagChipInput
              tags={langfuseForm.sampling_include_tags}
              onChange={(tags) => update({ sampling_include_tags: tags })}
            />
          </SettingsItem>
          <SettingsItem title="Exclude Tags" description="Never export traces with these tags.">
            <TagChipInput
              tags={langfuseForm.sampling_exclude_tags}
              onChange={(tags) => update({ sampling_exclude_tags: tags })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Redaction">
          <SettingsItem title="Enabled" description="Redact sensitive patterns before export.">
            <Checkbox
              checked={langfuseForm.redaction_enabled}
              onCheckedChange={(c) => update({ redaction_enabled: c === true })}
            />
          </SettingsItem>
          <SettingsItem title="Replacement" description="Text to replace matched patterns.">
            <Input
              className="w-[180px]"
              value={langfuseForm.redaction_replacement}
              placeholder="[REDACTED]"
              onChange={(e) => update({ redaction_replacement: e.target.value })}
            />
          </SettingsItem>
          <SettingsItem title="Patterns" description="Regex patterns for redaction.">
            <TagChipInput
              tags={langfuseForm.redaction_patterns}
              onChange={(tags) => update({ redaction_patterns: tags })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Retry Policy">
          <SettingsItem title="Max Retries">
            <Input
              numeric
              type="number"
              min={0}
              className="w-[100px]"
              value={langfuseForm.retry_max_retries}
              onChange={(e) => update({ retry_max_retries: Number(e.target.value) || 3 })}
            />
          </SettingsItem>
          <SettingsItem title="Initial Backoff (ms)">
            <Input
              numeric
              type="number"
              min={100}
              className="w-[120px]"
              value={langfuseForm.retry_initial_backoff_ms}
              onChange={(e) => update({ retry_initial_backoff_ms: Number(e.target.value) || 1000 })}
            />
          </SettingsItem>
          <SettingsItem title="Max Backoff (ms)">
            <Input
              numeric
              type="number"
              min={1000}
              className="w-[120px]"
              value={langfuseForm.retry_max_backoff_ms}
              onChange={(e) => update({ retry_max_backoff_ms: Number(e.target.value) || 30000 })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Circuit Breaker">
          <SettingsItem title="Failure Threshold" description="Consecutive failures before opening circuit.">
            <Input
              numeric
              type="number"
              min={1}
              className="w-[100px]"
              value={langfuseForm.circuit_breaker_failure_threshold}
              onChange={(e) => update({ circuit_breaker_failure_threshold: Number(e.target.value) || 5 })}
            />
          </SettingsItem>
          <SettingsItem title="Recovery Timeout (secs)" description="Seconds before attempting recovery.">
            <Input
              numeric
              type="number"
              min={1}
              className="w-[120px]"
              value={langfuseForm.circuit_breaker_recovery_timeout_secs}
              onChange={(e) => update({ circuit_breaker_recovery_timeout_secs: Number(e.target.value) || 60 })}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Feedback Import">
          <SettingsItem title="Import Enabled" description="Periodically import scores from Langfuse.">
            <Checkbox
              checked={langfuseForm.feedback_import_enabled}
              onCheckedChange={(c) => update({ feedback_import_enabled: c === true })}
            />
          </SettingsItem>
          <SettingsItem title="Poll Interval (secs)">
            <Input
              numeric
              type="number"
              min={10}
              className="w-[120px]"
              value={langfuseForm.feedback_poll_interval_secs}
              onChange={(e) => update({ feedback_poll_interval_secs: Number(e.target.value) || 300 })}
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
