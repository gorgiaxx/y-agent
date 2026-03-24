// ---------------------------------------------------------------------------
// SettingsPanel -- Orchestrator that delegates to tab-specific components.
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { GuiConfig } from '../../types';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { SESSION_SCHEMA, BROWSER_SCHEMA, RUNTIME_SCHEMA } from '../../utils/settingsSchemas';

import type {
  ProviderFormData,
  SessionFormData,
  RuntimeFormData,
  BrowserFormData,
  McpServerFormData,
} from './settingsTypes';
import {
  DEFAULT_SESSION_FORM,
  DEFAULT_RUNTIME_FORM,
  DEFAULT_BROWSER_FORM,
  providersToToml,
  mcpServersToJson,
  TAB_LABELS,
  CONFIG_SECTIONS,
} from './settingsTypes';

import { GeneralTab } from './GeneralTab';
import { ProvidersTab } from './ProvidersTab';
import { SessionTab } from './SessionTab';
import { RuntimeTab } from './RuntimeTab';
import { BrowserTab } from './BrowserTab';
import { McpTab } from './McpTab';
import { PromptsTab } from './PromptsTab';
import { AboutTab } from './AboutTab';
import { TomlEditorTab } from './TomlEditorTab';
import { Button } from '../ui';

import './SettingsPanel.css';

interface SettingsPanelProps {
  config: GuiConfig;
  activeTab: SettingsTab;
  onSave: (updates: Partial<GuiConfig>) => void;
  loadSection: (section: string) => Promise<string>;
  saveSection: (section: string, content: string) => Promise<void>;
  reloadConfig: () => Promise<string>;
}

export type SettingsTab = 'general' | 'providers' | 'session' | 'runtime' | 'browser' | 'mcp' | 'storage' | 'hooks' | 'tools' | 'guardrails' | 'knowledge' | 'prompts' | 'about';

export function SettingsPanel({
  config,
  activeTab,
  onSave,
  loadSection,
  saveSection,
  reloadConfig,
}: SettingsPanelProps) {
  const [localConfig, setLocalConfig] = useState<GuiConfig>({ ...config });
  const [toast, setToast] = useState<{ message: string; type: 'success' | 'error' } | null>(null);

  // ---------------------------------------------------------------------------
  // Lifted state for structured-form sections (needed by unified save).
  // ---------------------------------------------------------------------------

  // Providers
  const [providersList, setProvidersList] = useState<ProviderFormData[]>([]);
  const [providersMeta, setProvidersMeta] = useState('');
  const [dirtyProviders, setDirtyProviders] = useState(false);

  // Session
  const [sessionForm, setSessionForm] = useState<SessionFormData>({ ...DEFAULT_SESSION_FORM });
  const [dirtySession, setDirtySession] = useState(false);
  const [rawSessionToml, setRawSessionToml] = useState<string | undefined>(undefined);

  // Runtime
  const [runtimeForm, setRuntimeForm] = useState<RuntimeFormData>({ ...DEFAULT_RUNTIME_FORM });
  const [dirtyRuntime, setDirtyRuntime] = useState(false);
  const [rawRuntimeToml, setRawRuntimeToml] = useState<string | undefined>(undefined);

  // Browser
  const [browserForm, setBrowserForm] = useState<BrowserFormData>({ ...DEFAULT_BROWSER_FORM });
  const [dirtyBrowser, setDirtyBrowser] = useState(false);
  const [rawBrowserToml, setRawBrowserToml] = useState<string | undefined>(undefined);

  // MCP
  const [mcpServersList, setMcpServersList] = useState<McpServerFormData[]>([]);
  const [dirtyMcp, setDirtyMcp] = useState(false);

  // Raw TOML drafts (hooks, tools, storage, guardrails, knowledge)
  const [tomlDraftsBySection, setTomlDraftsBySection] = useState<Record<string, string>>({});

  // Prompts
  const [dirtyPrompts, setDirtyPrompts] = useState<Record<string, string>>({});

  // Whether the unified Save Changes is currently writing.
  const [saving, setSaving] = useState(false);

  // ---------------------------------------------------------------------------
  // Unified save
  // ---------------------------------------------------------------------------

  const handleSave = useCallback(async () => {
    setSaving(true);
    const errors: string[] = [];

    if (dirtyProviders) {
      try {
        const body = providersToToml(providersList);
        const toml = providersMeta ? `${providersMeta}${body}` : body;
        await saveSection('providers', toml);
        setDirtyProviders(false);
      } catch (e) { errors.push(`providers: ${e}`); }
    }

    if (dirtySession) {
      try {
        const toml = mergeIntoRawToml(rawSessionToml, sessionForm as unknown as Record<string, unknown>, SESSION_SCHEMA);
        await saveSection('session', toml);
        setRawSessionToml(toml);
        setDirtySession(false);
      } catch (e) { errors.push(`session: ${e}`); }
    }

    if (dirtyRuntime) {
      try {
        const toml = mergeIntoRawToml(rawRuntimeToml, runtimeForm as unknown as Record<string, unknown>, RUNTIME_SCHEMA);
        await saveSection('runtime', toml);
        setRawRuntimeToml(toml);
        setDirtyRuntime(false);
      } catch (e) { errors.push(`runtime: ${e}`); }
    }

    if (dirtyBrowser) {
      try {
        const toml = mergeIntoRawToml(rawBrowserToml, browserForm as unknown as Record<string, unknown>, BROWSER_SCHEMA);
        await saveSection('browser', toml);
        setRawBrowserToml(toml);
        setDirtyBrowser(false);
      } catch (e) { errors.push(`browser: ${e}`); }
    }

    if (dirtyMcp) {
      try {
        const json = mcpServersToJson(mcpServersList);
        await invoke('mcp_config_save', { content: json });
        setDirtyMcp(false);
      } catch (e) { errors.push(`mcp: ${e}`); }
    }

    for (const [section, content] of Object.entries(tomlDraftsBySection)) {
      try {
        await saveSection(section, content);
      } catch (e) { errors.push(`${section}: ${e}`); }
    }
    if (errors.length === 0) {
      setTomlDraftsBySection({});
    }

    // Save dirty prompt files.
    for (const [filename, content] of Object.entries(dirtyPrompts)) {
      try {
        await invoke('prompt_save', { filename, content });
      } catch (e) { errors.push(`prompt ${filename}: ${e}`); }
    }
    if (errors.length === 0) {
      setDirtyPrompts({});
    }

    setSaving(false);

    if (errors.length > 0) {
      setToast({ message: `Save failed: ${errors.join('; ')}`, type: 'error' });
      return;
    }

    // Hot-reload backend config so saved changes take effect immediately.
    try {
      await reloadConfig();
    } catch (e) {
      console.warn('Config reload after save failed:', e);
    }

    onSave(localConfig);
    setToast({ message: 'Settings saved', type: 'success' });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    dirtyProviders, dirtySession, dirtyRuntime, dirtyBrowser, dirtyMcp,
    providersList, providersMeta, sessionForm, runtimeForm, browserForm, mcpServersList,
    rawSessionToml, rawRuntimeToml, rawBrowserToml,
    tomlDraftsBySection, dirtyPrompts, saveSection, reloadConfig, localConfig, onSave,
  ]);

  // Auto-dismiss toast.
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(timer);
  }, [toast]);

  // ---------------------------------------------------------------------------
  // Determine which tab type we're rendering.
  // ---------------------------------------------------------------------------

  const isStructuredTab = ['general', 'providers', 'session', 'runtime', 'browser', 'mcp', 'prompts', 'about'].includes(activeTab);
  const isTomlEditor = !isStructuredTab;

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div className="settings-panel">
      <div className="settings-action-bar">
        <h2 className="settings-action-bar-title">{TAB_LABELS[activeTab] ?? activeTab}</h2>
        <div className="settings-action-bar-actions">
          <Button variant="primary" onClick={handleSave} disabled={saving}>
            {saving ? 'Saving...' : 'Save Changes'}
          </Button>
        </div>
      </div>

      <div className="settings-content">
        {activeTab === 'general' && (
          <GeneralTab
            localConfig={localConfig}
            setLocalConfig={setLocalConfig}
            setToast={setToast}
          />
        )}

        {activeTab === 'providers' && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                {CONFIG_SECTIONS.find((s) => s.key === activeTab)?.label ?? activeTab}
              </h3>
            </div>
            <ProvidersTab
              loadSection={loadSection}
              setToast={setToast}
              providersList={providersList}
              setProvidersList={setProvidersList}
              providersMeta={providersMeta}
              setProvidersMeta={setProvidersMeta}
              setDirtyProviders={setDirtyProviders}
            />
          </div>
        )}

        {activeTab === 'session' && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                Session
              </h3>
            </div>
            <SessionTab
              loadSection={loadSection}
              sessionForm={sessionForm}
              setSessionForm={setSessionForm}
              setDirtySession={setDirtySession}
              setRawSessionToml={setRawSessionToml}
            />
          </div>
        )}

        {activeTab === 'runtime' && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                Runtime
              </h3>
            </div>
            <RuntimeTab
              loadSection={loadSection}
              runtimeForm={runtimeForm}
              setRuntimeForm={setRuntimeForm}
              setDirtyRuntime={setDirtyRuntime}
              setRawRuntimeToml={setRawRuntimeToml}
            />
          </div>
        )}

        {activeTab === 'browser' && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                Browser
              </h3>
            </div>
            <BrowserTab
              loadSection={loadSection}
              browserForm={browserForm}
              setBrowserForm={setBrowserForm}
              setDirtyBrowser={setDirtyBrowser}
              setRawBrowserToml={setRawBrowserToml}
            />
          </div>
        )}

        {activeTab === 'mcp' && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                MCP Servers
              </h3>
            </div>
            <McpTab
              mcpServersList={mcpServersList}
              setMcpServersList={setMcpServersList}
              setDirtyMcp={setDirtyMcp}
            />
          </div>
        )}

        {activeTab === 'prompts' && (
          <PromptsTab
            setToast={setToast}
            dirtyPrompts={dirtyPrompts}
            setDirtyPrompts={setDirtyPrompts}
          />
        )}

        {activeTab === 'about' && <AboutTab />}

        {isTomlEditor && (
          <div className="settings-section">
            <div className="settings-header">
              <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                {CONFIG_SECTIONS.find((s) => s.key === activeTab)?.label ?? activeTab}
              </h3>
              <TomlEditorTab
                activeTab={activeTab}
                loadSection={loadSection}
                setToast={setToast}
                tomlDraftsBySection={tomlDraftsBySection}
                setTomlDraftsBySection={setTomlDraftsBySection}
              />
            </div>
          </div>
        )}
      </div>

      {/* Toast notification */}
      {toast && (
        <div className={`settings-toast ${toast.type}`}>
          {toast.message}
        </div>
      )}
    </div>
  );
}
