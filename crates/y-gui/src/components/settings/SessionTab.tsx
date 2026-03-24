// ---------------------------------------------------------------------------
// SessionTab -- Session configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SessionFormData } from './settingsTypes';
import { jsonToSession, DEFAULT_SESSION_FORM } from './settingsTypes';

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
      } catch {
        setRawSessionToml(undefined);
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

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  return (
    <div className="settings-form-wrap">
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Max Tree Depth</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            value={sessionForm.max_depth}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_depth: Number(e.target.value) || 16 }); setDirtySession(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Max Active per Root</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            value={sessionForm.max_active_per_root}
            onChange={(e) => { setSessionForm({ ...sessionForm, max_active_per_root: Number(e.target.value) || 8 }); setDirtySession(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Compaction Threshold (%)</label>
          <input
            className="pf-input pf-input-num"
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
            <input
              type="checkbox"
              className="form-checkbox"
              checked={sessionForm.auto_archive_merged}
              onChange={(e) => { setSessionForm({ ...sessionForm, auto_archive_merged: e.target.checked }); setDirtySession(true); }}
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
            <input
              type="checkbox"
              className="form-checkbox"
              checked={sessionForm.pruning_enabled}
              onChange={(e) => { setSessionForm({ ...sessionForm, pruning_enabled: e.target.checked }); setDirtySession(true); }}
            />
            {' '}Enable Pruning
          </label>
        </div>
        <div className="pf-field">
          <label className="pf-label">Strategy</label>
          <select
            className="form-select"
            style={{ maxWidth: 'none' }}
            value={sessionForm.pruning_strategy}
            onChange={(e) => { setSessionForm({ ...sessionForm, pruning_strategy: e.target.value }); setDirtySession(true); }}
          >
            <option value="auto">Auto (retry + progressive)</option>
            <option value="retry_only">Retry Only (zero LLM cost)</option>
            <option value="progressive_only">Progressive Only (LLM summarization)</option>
          </select>
        </div>
      </div>
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Token Threshold</label>
          <input
            className="pf-input pf-input-num"
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
          <input
            className="pf-input pf-input-num"
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
            <input
              type="checkbox"
              className="form-checkbox"
              checked={sessionForm.pruning_progressive_preserve_identifiers}
              onChange={(e) => { setSessionForm({ ...sessionForm, pruning_progressive_preserve_identifiers: e.target.checked }); setDirtySession(true); }}
            />
            {' '}Preserve identifiers in progressive summaries
          </label>
        </div>
      </div>
    </div>
  );
}
