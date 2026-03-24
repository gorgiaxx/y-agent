// ---------------------------------------------------------------------------
// ToolsTab -- Tool Registry configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ToolsFormData } from './settingsTypes';
import { jsonToTools } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { serializeToml } from '../../utils/tomlUtils';
import { TOOLS_SCHEMA } from '../../utils/settingsSchemas';
import { Checkbox } from '../ui';

interface ToolsTabProps {
  loadSection: (section: string) => Promise<string>;
  toolsForm: ToolsFormData;
  setToolsForm: React.Dispatch<React.SetStateAction<ToolsFormData>>;
  setDirtyTools: React.Dispatch<React.SetStateAction<boolean>>;
  setRawToolsToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function ToolsTab({
  loadSection,
  toolsForm,
  setToolsForm,
  setDirtyTools,
  setRawToolsToml,
}: ToolsTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const json = allConfig?.tools ?? {};
      setToolsForm(jsonToTools(json));
      try {
        const raw = await loadSection('tools');
        setRawToolsToml(raw);
        setRawContent(raw);
      } catch {
        setRawToolsToml(undefined);
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setToolsForm, setRawToolsToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(serializeToml(toolsForm as unknown as Record<string, unknown>, TOOLS_SCHEMA));
    }
    setRawMode(next);
  }, [toolsForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
            <span className="settings-header-with-toggle">Tools <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawToolsToml(val);
            setDirtyTools(true);
          }}
          placeholder="No tools.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
          <span className="settings-header-with-toggle">Tools <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">Max Active Tools</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={1}
              max={200}
              value={toolsForm.max_active}
              onChange={(e) => { setToolsForm({ ...toolsForm, max_active: Number(e.target.value) || 20 }); setDirtyTools(true); }}
            />
            <span className="pf-hint">Max fully-loaded tools per session.</span>
          </div>
          <div className="pf-field">
            <label className="pf-label">Search Limit</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={1}
              max={100}
              value={toolsForm.search_limit}
              onChange={(e) => { setToolsForm({ ...toolsForm, search_limit: Number(e.target.value) || 10 }); setDirtyTools(true); }}
            />
            <span className="pf-hint">Max results for tool_search.</span>
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <Checkbox
                checked={toolsForm.allow_dynamic_tools}
                onCheckedChange={(c) => { setToolsForm({ ...toolsForm, allow_dynamic_tools: c === true }); setDirtyTools(true); }}
              />
              {' '}Allow Dynamic Tools
            </label>
            <span className="pf-hint">Whether agents can create tools dynamically at runtime.</span>
          </div>
        </div>
      </div>
    </>
  );
}
