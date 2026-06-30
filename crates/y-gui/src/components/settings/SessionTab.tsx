// ---------------------------------------------------------------------------
// SessionTab -- Session configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { AppConfigResponse } from '../../types';
import type { SessionFormData, RetryFormData } from './settingsTypes';
import { jsonToSession, jsonToRetry } from './settingsTypes';
import { RawTomlEditor, RawModeToggle, SettingsActionSlot } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { SESSION_SCHEMA } from '../../utils/settingsSchemas';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, Switch, SettingsGroup, SettingsItem } from '../ui';

interface SessionTabProps {
  loadSection: (section: string) => Promise<string>;
  sessionForm: SessionFormData;
  setSessionForm: React.Dispatch<React.SetStateAction<SessionFormData>>;
  setDirtySession: React.Dispatch<React.SetStateAction<boolean>>;
  setRawSessionToml: React.Dispatch<React.SetStateAction<string | undefined>>;
  retryForm: RetryFormData;
  setRetryForm: React.Dispatch<React.SetStateAction<RetryFormData>>;
  setDirtyProviders: React.Dispatch<React.SetStateAction<boolean>>;
}

export function SessionTab({
  loadSection,
  sessionForm,
  setSessionForm,
  setDirtySession,
  setRawSessionToml,
  retryForm,
  setRetryForm,
  setDirtyProviders,
}: SessionTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadSessionForm = useCallback(async () => {
    setLoading(true);
    try {
            const allConfig = await transport.invoke<AppConfigResponse>('config_get');
      const sessionJson = allConfig?.session ?? {};
      setSessionForm(jsonToSession(sessionJson));
      setRetryForm(jsonToRetry(allConfig));
      // Cache raw TOML for comment preservation.
      try {
        const raw = await loadSection('session');
        setRawSessionToml(raw);
        cachedRawToml.current = raw;
      } catch {
        setRawSessionToml(undefined);
        cachedRawToml.current = undefined;
      }
    } catch {
      // Use defaults if section not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setSessionForm, setRetryForm, setRawSessionToml]);

  useEffect(() => {
    loadSessionForm();
  }, [loadSessionForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, sessionForm as unknown as Record<string, unknown>, SESSION_SCHEMA));
    }
    setRawMode(next);
  }, [sessionForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  const updateRetry = (patch: Partial<RetryFormData>) => {
    setRetryForm((prev) => ({ ...prev, ...patch }));
    setDirtyProviders(true);
  };

  if (rawMode) {
    return (
      <>
        <SettingsActionSlot><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></SettingsActionSlot>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawSessionToml(val);
            setDirtySession(true);
          }}
          placeholder="No session.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
    <SettingsActionSlot><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></SettingsActionSlot>
    <div className="settings-form-wrap">
      <SettingsGroup title="Session Tree">
        <SettingsItem title="Max Tree Depth">
          <Input
            numeric
            type="number"
            min={1}
            className="w-[100px]"
            value={sessionForm.max_depth}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_depth: Number(e.target.value) || 16 }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Max Active per Root">
          <Input
            numeric
            type="number"
            min={1}
            className="w-[100px]"
            value={sessionForm.max_active_per_root}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_active_per_root: Number(e.target.value) || 8 }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Compaction Threshold (%)">
          <Input
            numeric
            type="number"
            min={50}
            max={95}
            step={5}
            className="w-[100px]"
            value={sessionForm.compaction_threshold_pct}
            onChange={(e) => { setSessionForm({ ...sessionForm, compaction_threshold_pct: Math.min(95, Math.max(50, Number(e.target.value) || 85)) }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Auto-archive merged sessions">
          <Checkbox
            checked={sessionForm.auto_archive_merged}
            onCheckedChange={(c) => { setSessionForm({ ...sessionForm, auto_archive_merged: c === true }); setDirtySession(true); }}
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Context Pruning">
        <SettingsItem title="Enable Pruning">
          <Checkbox
            checked={sessionForm.pruning_enabled}
            onCheckedChange={(c) => { setSessionForm({ ...sessionForm, pruning_enabled: c === true }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Strategy">
          <Select
            value={sessionForm.pruning_strategy}
            onValueChange={(val) => { setSessionForm({ ...sessionForm, pruning_strategy: val }); setDirtySession(true); }}
          >
            <SelectTrigger className="w-[220px]">
              <SelectValue placeholder="Select strategy" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">Auto (retry + progressive)</SelectItem>
              <SelectItem value="retry_only">Retry Only (zero LLM cost)</SelectItem>
              <SelectItem value="progressive_only">Progressive Only (LLM summarization)</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Token Threshold" description="Min token growth before pruning triggers">
          <Input
            numeric
            type="number"
            min={500}
            step={500}
            className="w-[100px]"
            value={sessionForm.pruning_token_threshold}
            onChange={(e) => { setSessionForm({ ...sessionForm, pruning_token_threshold: Number(e.target.value) || 2000 }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Max Retries">
          <Input
            numeric
            type="number"
            min={0}
            max={10}
            className="w-[100px]"
            value={sessionForm.pruning_progressive_max_retries}
            onChange={(e) => { setSessionForm({ ...sessionForm, pruning_progressive_max_retries: Number(e.target.value) || 2 }); setDirtySession(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Preserve identifiers in progressive summaries">
          <Checkbox
            checked={sessionForm.pruning_progressive_preserve_identifiers}
            onCheckedChange={(c) => { setSessionForm({ ...sessionForm, pruning_progressive_preserve_identifiers: c === true }); setDirtySession(true); }}
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup
        title="Retry Policy"
        description="Automatically retry timeout / 5xx provider errors (e.g. HTTP 504) against the same provider before it is frozen. Applies to all providers."
      >
        <SettingsItem title="Enable automatic retry">
          <Switch
            checked={retryForm.enabled}
            onCheckedChange={(checked) => updateRetry({ enabled: checked })}
          />
        </SettingsItem>
        <SettingsItem title="Max retries" description="Extra attempts after the first failure">
          <Input
            numeric
            type="number"
            min={0}
            className="w-[100px]"
            disabled={!retryForm.enabled}
            value={retryForm.max_retries}
            onChange={(e) => updateRetry({ max_retries: Math.max(0, Number(e.target.value) || 0) })}
          />
        </SettingsItem>
        <SettingsItem title="Initial delay (ms)" description="Delay before the first retry">
          <Input
            numeric
            type="number"
            min={0}
            step={100}
            className="w-[120px]"
            disabled={!retryForm.enabled}
            value={retryForm.initial_delay_ms}
            onChange={(e) => updateRetry({ initial_delay_ms: Math.max(0, Number(e.target.value) || 0) })}
          />
        </SettingsItem>
        <SettingsItem title="Max delay (ms)" description="Upper bound for any single backoff delay">
          <Input
            numeric
            type="number"
            min={0}
            step={1000}
            className="w-[120px]"
            disabled={!retryForm.enabled}
            value={retryForm.max_delay_ms}
            onChange={(e) => updateRetry({ max_delay_ms: Math.max(0, Number(e.target.value) || 0) })}
          />
        </SettingsItem>
        <SettingsItem title="Backoff" description="How the delay grows across retries">
          <Select
            value={retryForm.backoff}
            onValueChange={(val) => updateRetry({ backoff: val === 'fixed' ? 'fixed' : 'exponential' })}
            disabled={!retryForm.enabled}
          >
            <SelectTrigger className="w-[200px]">
              <SelectValue placeholder="Select backoff" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="exponential">Exponential (1s, 2s, 4s, ...)</SelectItem>
              <SelectItem value="fixed">Fixed (constant interval)</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
      </SettingsGroup>
    </div>
    </>
  );
}
