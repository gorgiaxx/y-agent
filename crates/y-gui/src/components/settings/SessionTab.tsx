// ---------------------------------------------------------------------------
// SessionTab -- Session configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SessionFormData } from './settingsTypes';
import { jsonToSession } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { SESSION_SCHEMA } from '../../utils/settingsSchemas';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input } from '../ui';

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
      const allConfig = await invoke<any>('config_get');
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
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
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
      <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
        <span className="settings-header-with-toggle">Session <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <div className="settings-form-wrap">
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Max Tree Depth</label>
          <Input
            numeric
            type="number"
            min={1}
            value={sessionForm.max_depth}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_depth: Number(e.target.value) || 16 }); setDirtySession(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Max Active per Root</label>
          <Input
            numeric
            type="number"
            min={1}
            value={sessionForm.max_active_per_root}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_active_per_root: Number(e.target.value) || 8 }); setDirtySession(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Compaction Threshold (%)</label>
          <Input
            numeric
            type="number"
            min={50}
            max={95}
            step={5}
            value={sessionForm.compaction_threshold_pct}
            onChange={(e) => { setSessionForm({ ...sessionForm, compaction_threshold_pct: Math.min(95, Math.max(50, Number(e.target.value) || 85)) }); setDirtySession(true); }}
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <Checkbox
              checked={sessionForm.auto_archive_merged}
              onCheckedChange={(c) => { setSessionForm({ ...sessionForm, auto_archive_merged: c === true }); setDirtySession(true); }}
            />
            {' '}Auto-archive merged sessions
          </label>
        </div>
      </div>

      {/* Pruning configuration */}
      <div className="pf-section-divider">
        <span className="pf-section-title">Context Pruning</span>
      </div>
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">
            <Checkbox
              checked={sessionForm.pruning_enabled}
              onCheckedChange={(c) => { setSessionForm({ ...sessionForm, pruning_enabled: c === true }); setDirtySession(true); }}
            />
            {' '}Enable Pruning
          </label>
        </div>
        <div className="pf-field">
          <label className="pf-label">Strategy</label>
          <Select
            value={sessionForm.pruning_strategy}
            onValueChange={(val) => { setSessionForm({ ...sessionForm, pruning_strategy: val }); setDirtySession(true); }}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select strategy" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">Auto (retry + progressive)</SelectItem>
              <SelectItem value="retry_only">Retry Only (zero LLM cost)</SelectItem>
              <SelectItem value="progressive_only">Progressive Only (LLM summarization)</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Token Threshold</label>
          <Input
            numeric
            type="number"
            min={500}
            step={500}
            value={sessionForm.pruning_token_threshold}
            onChange={(e) => { setSessionForm({ ...sessionForm, pruning_token_threshold: Number(e.target.value) || 2000 }); setDirtySession(true); }}
          />
          <span className="pf-hint">Min token growth before pruning triggers</span>
        </div>
        <div className="pf-field">
          <label className="pf-label">Max Retries</label>
          <Input
            numeric
            type="number"
            min={0}
            max={10}
            value={sessionForm.pruning_progressive_max_retries}
            onChange={(e) => { setSessionForm({ ...sessionForm, pruning_progressive_max_retries: Number(e.target.value) || 2 }); setDirtySession(true); }}
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <Checkbox
              checked={sessionForm.pruning_progressive_preserve_identifiers}
              onCheckedChange={(c) => { setSessionForm({ ...sessionForm, pruning_progressive_preserve_identifiers: c === true }); setDirtySession(true); }}
            />
            {' '}Preserve identifiers in progressive summaries
          </label>
        </div>
      </div>
    </div>
    </>
  );
}
