// ---------------------------------------------------------------------------
// BrowserTab -- Browser (CDP) configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TagChipInput } from './TagChipInput';
import type { BrowserFormData } from './settingsTypes';
import { jsonToBrowser } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { BROWSER_SCHEMA } from '../../utils/settingsSchemas';
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
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadBrowserForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const browserJson = allConfig?.browser ?? {};
      setBrowserForm(jsonToBrowser(browserJson));
      // Cache raw TOML for comment preservation.
      try {
        const raw = await loadSection('browser');
        setRawBrowserToml(raw);
        cachedRawToml.current = raw;
      } catch {
        setRawBrowserToml(undefined);
        cachedRawToml.current = undefined;
      }
    } catch {
      // Use defaults if section not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setBrowserForm, setRawBrowserToml]);

  useEffect(() => {
    loadBrowserForm();
  }, [loadBrowserForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, browserForm as unknown as Record<string, unknown>, BROWSER_SCHEMA));
    }
    setRawMode(next);
  }, [browserForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle">Browser <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawBrowserToml(val);
            setDirtyBrowser(true);
          }}
          placeholder="No browser.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
    <div className="settings-header">
      <h3 className="section-title section-title--flush">
        <span className="settings-header-with-toggle">Browser <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <div className="settings-form-wrap">
      <SettingsGroup title="General">
        <SettingsItem
          title="Enable browser tool"
          description="When disabled, the agent cannot use browser automation"
        >
          <Checkbox
            checked={browserForm.enabled}
            onCheckedChange={(c) => { setBrowserForm({ ...browserForm, enabled: c === true }); setDirtyBrowser(true); }}
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
            onValueChange={(val) => { setBrowserForm({ ...browserForm, launch_mode: val as BrowserFormData['launch_mode'] }); setDirtyBrowser(true); }}
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
              onChange={(e) => { setBrowserForm({ ...browserForm, chrome_path: e.target.value }); setDirtyBrowser(true); }}
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
              onChange={(e) => { setBrowserForm({ ...browserForm, local_cdp_port: Number(e.target.value) || 9222 }); setDirtyBrowser(true); }}
            />
          </SettingsItem>
          <SettingsItem
            title="Use system user profile"
            description="Use your Chrome profile (bookmarks, cookies, extensions) instead of a clean temporary profile."
          >
            <Checkbox
              checked={browserForm.use_user_profile}
              onCheckedChange={(c) => { setBrowserForm({ ...browserForm, use_user_profile: c === true }); setDirtyBrowser(true); }}
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
              onChange={(e) => { setBrowserForm({ ...browserForm, cdp_url: e.target.value }); setDirtyBrowser(true); }}
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
            onChange={(e) => { setBrowserForm({ ...browserForm, timeout_ms: Number(e.target.value) || 30000 }); setDirtyBrowser(true); }}
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
            onChange={(e) => { setBrowserForm({ ...browserForm, max_screenshot_dim: Number(e.target.value) || 4096 }); setDirtyBrowser(true); }}
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
            onChange={(next) => { setBrowserForm({ ...browserForm, allowed_domains: next }); setDirtyBrowser(true); }}
          />
        </SettingsItem>
        <SettingsItem title="Block private networks (SSRF protection)">
          <Checkbox
            checked={browserForm.block_private_networks}
            onCheckedChange={(c) => { setBrowserForm({ ...browserForm, block_private_networks: c === true }); setDirtyBrowser(true); }}
          />
        </SettingsItem>
      </SettingsGroup>
    </div>
    </>
  );
}
