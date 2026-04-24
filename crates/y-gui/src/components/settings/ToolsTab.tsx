// ---------------------------------------------------------------------------
// ToolsTab -- Tool Registry configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { ToolsFormData } from './settingsTypes';
import { jsonToTools } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { TOOLS_SCHEMA } from '../../utils/settingsSchemas';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

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
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await transport.invoke<any>('config_get');
      const json = allConfig?.tools ?? {};
      setToolsForm(jsonToTools(json));
      try {
        const raw = await loadSection('tools');
        setRawToolsToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawToolsToml(undefined);
        cachedRawToml.current = undefined;
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
      setRawContent(mergeIntoRawToml(cachedRawToml.current, toolsForm as unknown as Record<string, unknown>, TOOLS_SCHEMA));
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
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
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
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Tool Registry">
          <SettingsItem title="Max Active Tools" description="Max fully-loaded tools per session.">
            <Input
              numeric
              type="number"
              min={1}
              max={200}
              className="w-[100px]"
              value={toolsForm.max_active}
              onChange={(e) => { setToolsForm({ ...toolsForm, max_active: Number(e.target.value) || 20 }); setDirtyTools(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Search Limit" description="Max results for ToolSearch.">
            <Input
              numeric
              type="number"
              min={1}
              max={100}
              className="w-[100px]"
              value={toolsForm.search_limit}
              onChange={(e) => { setToolsForm({ ...toolsForm, search_limit: Number(e.target.value) || 10 }); setDirtyTools(true); }}
            />
          </SettingsItem>
          <SettingsItem
            title="Allow Dynamic Tools"
            description="Whether agents can create tools dynamically at runtime."
          >
            <Checkbox
              checked={toolsForm.allow_dynamic_tools}
              onCheckedChange={(c) => { setToolsForm({ ...toolsForm, allow_dynamic_tools: c === true }); setDirtyTools(true); }}
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
