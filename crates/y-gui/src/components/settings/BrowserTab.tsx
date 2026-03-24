// ---------------------------------------------------------------------------
// BrowserTab -- Browser (CDP) configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TagChipInput } from './TagChipInput';
import type { BrowserFormData } from './settingsTypes';
import { jsonToBrowser } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { serializeToml } from '../../utils/tomlUtils';
import { BROWSER_SCHEMA } from '../../utils/settingsSchemas';

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
      } catch {
        setRawBrowserToml(undefined);
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
      setRawContent(serializeToml(browserForm as unknown as Record<string, unknown>, BROWSER_SCHEMA));
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
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
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
      <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
        <span className="settings-header-with-toggle">Browser <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <div className="settings-form-wrap">
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <input
              type="checkbox"
              className="form-checkbox"
              checked={browserForm.enabled}
              onChange={(e) => { setBrowserForm({ ...browserForm, enabled: e.target.checked }); setDirtyBrowser(true); }}
            />
            {' '}Enable browser tool
          </label>
          <span className="pf-hint">When disabled, the agent cannot use browser automation</span>
        </div>
      </div>

      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Launch Mode</label>
          <select
            className="form-select"
            style={{ maxWidth: 'none' }}
            value={browserForm.launch_mode}
            onChange={(e) => { setBrowserForm({ ...browserForm, launch_mode: e.target.value as BrowserFormData['launch_mode'] }); setDirtyBrowser(true); }}
          >
            <option value="auto_launch_headless">Auto Launch (Headless)</option>
            <option value="auto_launch_visible">Auto Launch (Visible Window)</option>
            <option value="remote">Remote CDP Endpoint</option>
          </select>
          <span className="pf-hint">
            {browserForm.launch_mode === 'remote'
              ? 'Connect to an already-running Chrome instance via CDP URL. You must start Chrome manually with --remote-debugging-port.'
              : browserForm.launch_mode === 'auto_launch_headless'
                ? 'Automatically launch Chrome in headless mode (no visible window). Recommended for most use cases.'
                : 'Automatically launch Chrome with a visible window. Useful for debugging and visual verification.'}
          </span>
        </div>
      </div>

      {browserForm.launch_mode !== 'remote' ? (
        /* Local Chrome mode */
        <>
        <div className="pf-row">
          <div className="pf-field" style={{ flex: 2 }}>
            <label className="pf-label">Chrome Path</label>
            <input
              className="pf-input"
              value={browserForm.chrome_path}
              onChange={(e) => { setBrowserForm({ ...browserForm, chrome_path: e.target.value }); setDirtyBrowser(true); }}
              placeholder="Auto-detect (leave empty)"
            />
            <span className="pf-hint">Path to Chrome/Chromium executable. Empty = auto-detect.</span>
          </div>
          <div className="pf-field" style={{ maxWidth: '140px' }}>
            <label className="pf-label">Debug Port</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={1024}
              max={65535}
              value={browserForm.local_cdp_port}
              onChange={(e) => { setBrowserForm({ ...browserForm, local_cdp_port: Number(e.target.value) || 9222 }); setDirtyBrowser(true); }}
            />
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <input
                type="checkbox"
                className="form-checkbox"
                checked={browserForm.use_user_profile}
                onChange={(e) => { setBrowserForm({ ...browserForm, use_user_profile: e.target.checked }); setDirtyBrowser(true); }}
              />
              {' '}Use system user profile
            </label>
            <span className="pf-hint">
              Use your Chrome profile (bookmarks, cookies, extensions, login sessions) instead of a clean temporary profile.
              Note: Chrome locks its profile directory -- close other Chrome instances before enabling this.
            </span>
          </div>
        </div>
        </>
      ) : (
        /* Remote CDP mode */
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">CDP Endpoint URL</label>
            <input
              className="pf-input"
              value={browserForm.cdp_url}
              onChange={(e) => { setBrowserForm({ ...browserForm, cdp_url: e.target.value }); setDirtyBrowser(true); }}
              placeholder="http://127.0.0.1:9222"
            />
            <span className="pf-hint">Remote Chrome DevTools Protocol endpoint. Supports http://, https://, ws://, wss://</span>
          </div>
        </div>
      )}

      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Timeout (ms)</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1000}
            step={1000}
            value={browserForm.timeout_ms}
            onChange={(e) => { setBrowserForm({ ...browserForm, timeout_ms: Number(e.target.value) || 30000 }); setDirtyBrowser(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Max Screenshot Dimension (px)</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={256}
            step={256}
            value={browserForm.max_screenshot_dim}
            onChange={(e) => { setBrowserForm({ ...browserForm, max_screenshot_dim: Number(e.target.value) || 4096 }); setDirtyBrowser(true); }}
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Allowed Domains</label>
          <TagChipInput
            tags={browserForm.allowed_domains}
            onChange={(next) => { setBrowserForm({ ...browserForm, allowed_domains: next }); setDirtyBrowser(true); }}
          />
          <span className="pf-hint">Domains the browser can navigate to. Use * to allow all public domains. Empty = all blocked.</span>
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <input
              type="checkbox"
              className="form-checkbox"
              checked={browserForm.block_private_networks}
              onChange={(e) => { setBrowserForm({ ...browserForm, block_private_networks: e.target.checked }); setDirtyBrowser(true); }}
            />
            {' '}Block private networks (SSRF protection)
          </label>
        </div>
      </div>
    </div>
    </>
  );
}
