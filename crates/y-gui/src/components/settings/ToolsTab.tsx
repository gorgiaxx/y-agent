// ---------------------------------------------------------------------------
// ToolsTab -- Tool Registry configuration form
// ---------------------------------------------------------------------------

import type { ToolsFormData } from './settingsTypes';
import { jsonToTools } from './settingsTypes';
import { TOOLS_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
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
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'tools',
    schema: TOOLS_SCHEMA,
    configKey: 'tools',
    form: toolsForm,
    setForm: setToolsForm,
    setDirty: setDirtyTools,
    setRawToml: setRawToolsToml,
    jsonToForm: jsonToTools,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No tools.toml found. Content will be created on save."
      form={
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
                onChange={(e) => update({ max_active: Number(e.target.value) || 20 })}
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
                onChange={(e) => update({ search_limit: Number(e.target.value) || 10 })}
              />
            </SettingsItem>
            <SettingsItem
              title="Allow Dynamic Tools"
              description="Whether agents can create tools dynamically at runtime."
            >
              <Checkbox
                checked={toolsForm.allow_dynamic_tools}
                onCheckedChange={(c) => update({ allow_dynamic_tools: c === true })}
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
