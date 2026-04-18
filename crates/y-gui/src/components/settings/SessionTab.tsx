// ---------------------------------------------------------------------------
// SessionTab -- Session configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { SessionFormData } from './settingsTypes';
import { jsonToSession } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { SESSION_SCHEMA } from '../../utils/settingsSchemas';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

interface SessionTabProps {
  loadSection: (section: string) => Promise<string>;
  sessionForm: SessionFormData;
  setSessionForm: React.Dispatch<React.SetStateAction<SessionFormData>>;
  setDirtySession: React.Dispatch<React.SetStateAction<boolean>>;
  setRawSessionToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function SessionTab({
  loadSection,
  sessionForm,
  setSessionForm,
  setDirtySession,
  setRawSessionToml,
}: SessionTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadSessionForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await transport.invoke<any>('config_get');
      const sessionJson = allConfig?.session ?? {};
      setSessionForm(jsonToSession(sessionJson));
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
  }, [loadSection, setSessionForm, setRawSessionToml]);

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

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle">Session <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
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
    <div className="settings-header">
      <h3 className="section-title section-title--flush">
        <span className="settings-header-with-toggle">Session <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
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
    </div>
    </>
  );
}
