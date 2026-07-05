// ---------------------------------------------------------------------------
// BrowserTab -- Browser (CDP) configuration form
// ---------------------------------------------------------------------------

import { TagChipInput } from './TagChipInput';
import type { BrowserFormData } from './settingsTypes';
import { jsonToBrowser } from './settingsTypes';
import { BROWSER_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

interface BrowserTabProps {
  loadSection: (section: string) => Promise<string>;
  browserForm: BrowserFormData;
  setBrowserForm: React.Dispatch<React.SetStateAction<BrowserFormData>>;
  setDirtyBrowser: React.Dispatch<React.SetStateAction<boolean>>;
  setRawBrowserToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function BrowserTab({
  loadSection,
  browserForm,
  setBrowserForm,
  setDirtyBrowser,
  setRawBrowserToml,
}: BrowserTabProps) {
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'browser',
    schema: BROWSER_SCHEMA,
    configKey: 'browser',
    form: browserForm,
    setForm: setBrowserForm,
    setDirty: setDirtyBrowser,
    setRawToml: setRawBrowserToml,
    jsonToForm: jsonToBrowser,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No browser.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="General">
            <SettingsItem
              title="Enable browser tool"
              description="When disabled, the agent cannot use browser automation"
            >
              <Checkbox
                checked={browserForm.enabled}
                onCheckedChange={(c) => update({ enabled: c === true })}
              />
            </SettingsItem>
            <SettingsItem
              title="Launch Mode"
              description={browserForm.launch_mode === 'remote'
                ? 'Connect to an already-running Chrome instance via CDP URL.'
                : browserForm.launch_mode === 'auto_launch_headless'
                  ? 'Automatically launch Chrome in headless mode (no visible window).'
                  : 'Automatically launch Chrome with a visible window.'}
            >
              <Select
                value={browserForm.launch_mode}
                onValueChange={(val) => update({ launch_mode: val as BrowserFormData['launch_mode'] })}
              >
                <SelectTrigger className="w-[220px]">
                  <SelectValue placeholder="Select launch mode" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="auto_launch_headless">Auto Launch (Headless)</SelectItem>
                  <SelectItem value="auto_launch_visible">Auto Launch (Visible Window)</SelectItem>
                  <SelectItem value="remote">Remote CDP Endpoint</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
          </SettingsGroup>

          {browserForm.launch_mode !== 'remote' ? (
            <SettingsGroup title="Local Chrome">
              <SettingsItem
                title="Chrome Path"
                description="Path to Chrome/Chromium executable. Empty = auto-detect."
                wide
              >
                <Input
                  value={browserForm.chrome_path}
                  onChange={(e) => update({ chrome_path: e.target.value })}
                  placeholder="Auto-detect (leave empty)"
                />
              </SettingsItem>
              <SettingsItem title="Debug Port">
                <Input
                  numeric
                  type="number"
                  min={1024}
                  max={65535}
                  className="w-[100px]"
                  value={browserForm.local_cdp_port}
                  onChange={(e) => update({ local_cdp_port: Number(e.target.value) || 9222 })}
                />
              </SettingsItem>
              <SettingsItem
                title="Use system user profile"
                description="Use your Chrome profile (bookmarks, cookies, extensions) instead of a clean temporary profile."
              >
                <Checkbox
                  checked={browserForm.use_user_profile}
                  onCheckedChange={(c) => update({ use_user_profile: c === true })}
                />
              </SettingsItem>
            </SettingsGroup>
          ) : (
            <SettingsGroup title="Remote CDP">
              <SettingsItem
                title="CDP Endpoint URL"
                description="Remote Chrome DevTools Protocol endpoint. Supports http://, https://, ws://, wss://"
                wide
              >
                <Input
                  value={browserForm.cdp_url}
                  onChange={(e) => update({ cdp_url: e.target.value })}
                  placeholder="http://127.0.0.1:9222"
                />
              </SettingsItem>
            </SettingsGroup>
          )}

          <SettingsGroup title="Limits">
            <SettingsItem title="Timeout (ms)">
              <Input
                numeric
                type="number"
                min={1000}
                step={1000}
                className="w-[100px]"
                value={browserForm.timeout_ms}
                onChange={(e) => update({ timeout_ms: Number(e.target.value) || 30000 })}
              />
            </SettingsItem>
            <SettingsItem title="Max Screenshot Dimension (px)">
              <Input
                numeric
                type="number"
                min={256}
                step={256}
                className="w-[100px]"
                value={browserForm.max_screenshot_dim}
                onChange={(e) => update({ max_screenshot_dim: Number(e.target.value) || 4096 })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="Security">
            <SettingsItem
              title="Allowed Domains"
              description="Domains the browser can navigate to. Use * to allow all. Empty = all blocked."
              wide
            >
              <TagChipInput
                tags={browserForm.allowed_domains}
                onChange={(next) => update({ allowed_domains: next })}
              />
            </SettingsItem>
            <SettingsItem title="Block private networks (SSRF protection)">
              <Checkbox
                checked={browserForm.block_private_networks}
                onCheckedChange={(c) => update({ block_private_networks: c === true })}
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
