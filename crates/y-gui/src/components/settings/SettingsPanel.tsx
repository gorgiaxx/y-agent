// ---------------------------------------------------------------------------
// SettingsPanel -- Orchestrator that delegates to tab-specific components.
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { transport } from '../../lib';
import type { GuiConfig } from '../../types';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import {
  SESSION_SCHEMA, BROWSER_SCHEMA, RUNTIME_SCHEMA,
  STORAGE_SCHEMA, HOOKS_SCHEMA, TOOLS_SCHEMA, GUARDRAILS_SCHEMA, KNOWLEDGE_SCHEMA,
} from '../../utils/settingsSchemas';

import type {
  ProviderFormData,
  SessionFormData,
  RuntimeFormData,
  BrowserFormData,
  McpServerFormData,
  StorageFormData,
  HooksFormData,
  ToolsFormData,
  GuardrailsFormData,
  KnowledgeFormData,
} from './settingsTypes';
import {
  DEFAULT_SESSION_FORM,
  DEFAULT_RUNTIME_FORM,
  DEFAULT_BROWSER_FORM,
  DEFAULT_STORAGE_FORM,
  DEFAULT_HOOKS_FORM,
  DEFAULT_TOOLS_FORM,
  DEFAULT_GUARDRAILS_FORM,
  DEFAULT_KNOWLEDGE_FORM,
  providersToToml,
  mcpServersToJson,
  TAB_LABELS,
} from './settingsTypes';

import { GeneralTab } from './GeneralTab';
import { ProvidersTab } from './ProvidersTab';
import { SessionTab } from './SessionTab';
import { RuntimeTab } from './RuntimeTab';
import { BrowserTab } from './BrowserTab';
import { McpTab } from './McpTab';
import { StorageTab } from './StorageTab';
import { HooksTab } from './HooksTab';
import { ToolsTab } from './ToolsTab';
import { GuardrailsTab } from './GuardrailsTab';
import { KnowledgeTab } from './KnowledgeTab';
import { PromptsTab } from './PromptsTab';
import { AboutTab } from './AboutTab';
import { Button, Tabs, TabsContent } from '../ui';

import './SettingsPanel.css';
import './SettingsForm.css';

interface SettingsPanelProps {
  config: GuiConfig;
  activeTab: SettingsTab;
  onSave: (updates: Partial<GuiConfig>) => void;
  loadSection: (section: string) => Promise<string>;
  saveSection: (section: string, content: string) => Promise<void>;
  reloadConfig: () => Promise<string>;
  onRunWizard?: () => void;
}

export type SettingsTab = 'general' | 'providers' | 'session' | 'runtime' | 'browser' | 'mcp' | 'storage' | 'hooks' | 'tools' | 'guardrails' | 'knowledge' | 'prompts' | 'about';

export function SettingsPanel({
  config,
  activeTab,
  onSave,
  loadSection,
  saveSection,
  reloadConfig,
  onRunWizard,
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
  const [rawProvidersToml, setRawProvidersToml] = useState<string | undefined>(undefined);

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

  // Storage
  const [storageForm, setStorageForm] = useState<StorageFormData>({ ...DEFAULT_STORAGE_FORM });
  const [dirtyStorage, setDirtyStorage] = useState(false);
  const [rawStorageToml, setRawStorageToml] = useState<string | undefined>(undefined);

  // Hooks
  const [hooksForm, setHooksForm] = useState<HooksFormData>({ ...DEFAULT_HOOKS_FORM });
  const [dirtyHooks, setDirtyHooks] = useState(false);
  const [rawHooksToml, setRawHooksToml] = useState<string | undefined>(undefined);

  // Tools
  const [toolsForm, setToolsForm] = useState<ToolsFormData>({ ...DEFAULT_TOOLS_FORM });
  const [dirtyTools, setDirtyTools] = useState(false);
  const [rawToolsToml, setRawToolsToml] = useState<string | undefined>(undefined);

  // Guardrails
  const [guardrailsForm, setGuardrailsForm] = useState<GuardrailsFormData>({ ...DEFAULT_GUARDRAILS_FORM });
  const [dirtyGuardrails, setDirtyGuardrails] = useState(false);
  const [rawGuardrailsToml, setRawGuardrailsToml] = useState<string | undefined>(undefined);

  // Knowledge
  const [knowledgeForm, setKnowledgeForm] = useState<KnowledgeFormData>({ ...DEFAULT_KNOWLEDGE_FORM });
  const [dirtyKnowledge, setDirtyKnowledge] = useState(false);
  const [rawKnowledgeToml, setRawKnowledgeToml] = useState<string | undefined>(undefined);

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
        await transport.invoke('mcp_config_save', { content: json });
        setDirtyMcp(false);
      } catch (e) { errors.push(`mcp: ${e}`); }
    }

    if (dirtyStorage) {
      try {
        const toml = mergeIntoRawToml(rawStorageToml, storageForm as unknown as Record<string, unknown>, STORAGE_SCHEMA);
        await saveSection('storage', toml);
        setRawStorageToml(toml);
        setDirtyStorage(false);
      } catch (e) { errors.push(`storage: ${e}`); }
    }

    if (dirtyHooks) {
      try {
        const toml = mergeIntoRawToml(rawHooksToml, hooksForm as unknown as Record<string, unknown>, HOOKS_SCHEMA);
        await saveSection('hooks', toml);
        setRawHooksToml(toml);
        setDirtyHooks(false);
      } catch (e) { errors.push(`hooks: ${e}`); }
    }

    if (dirtyTools) {
      try {
        const toml = mergeIntoRawToml(rawToolsToml, toolsForm as unknown as Record<string, unknown>, TOOLS_SCHEMA);
        await saveSection('tools', toml);
        setRawToolsToml(toml);
        setDirtyTools(false);
      } catch (e) { errors.push(`tools: ${e}`); }
    }

    if (dirtyGuardrails) {
      try {
        const toml = mergeIntoRawToml(rawGuardrailsToml, guardrailsForm as unknown as Record<string, unknown>, GUARDRAILS_SCHEMA);
        await saveSection('guardrails', toml);
        setRawGuardrailsToml(toml);
        setDirtyGuardrails(false);
      } catch (e) { errors.push(`guardrails: ${e}`); }
    }

    if (dirtyKnowledge) {
      try {
        const toml = mergeIntoRawToml(rawKnowledgeToml, knowledgeForm as unknown as Record<string, unknown>, KNOWLEDGE_SCHEMA);
        await saveSection('knowledge', toml);
        setRawKnowledgeToml(toml);
        setDirtyKnowledge(false);
      } catch (e) { errors.push(`knowledge: ${e}`); }
    }

    // Save dirty prompt files.
    for (const [filename, content] of Object.entries(dirtyPrompts)) {
      try {
        await transport.invoke('prompt_save', { filename, content });
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
  }, [
    dirtyProviders, dirtySession, dirtyRuntime, dirtyBrowser, dirtyMcp,
    dirtyStorage, dirtyHooks, dirtyTools, dirtyGuardrails, dirtyKnowledge,
    providersList, providersMeta, sessionForm, runtimeForm, browserForm, mcpServersList,
    storageForm, hooksForm, toolsForm, guardrailsForm, knowledgeForm,
    rawSessionToml, rawRuntimeToml, rawBrowserToml,
    rawStorageToml, rawHooksToml, rawToolsToml, rawGuardrailsToml, rawKnowledgeToml,
    dirtyPrompts, saveSection, reloadConfig, localConfig, onSave,
  ]);

  // Auto-dismiss toast.
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(timer);
  }, [toast]);

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

      <Tabs value={activeTab} className="settings-content">
        <TabsContent value="general">
          <GeneralTab
            localConfig={localConfig}
            setLocalConfig={setLocalConfig}
            setToast={setToast}
            onRunWizard={onRunWizard}
          />
        </TabsContent>

        <TabsContent value="providers" className="settings-section">
          <ProvidersTab
            loadSection={loadSection}
            setToast={setToast}
            providersList={providersList}
            setProvidersList={setProvidersList}
            providersMeta={providersMeta}
            setProvidersMeta={setProvidersMeta}
            setDirtyProviders={setDirtyProviders}
            rawProvidersToml={rawProvidersToml}
            setRawProvidersToml={setRawProvidersToml}
          />
        </TabsContent>

        <TabsContent value="session" className="settings-section">
          <SessionTab
            loadSection={loadSection}
            sessionForm={sessionForm}
            setSessionForm={setSessionForm}
            setDirtySession={setDirtySession}
            setRawSessionToml={setRawSessionToml}
          />
        </TabsContent>

        <TabsContent value="runtime" className="settings-section">
          <RuntimeTab
            loadSection={loadSection}
            runtimeForm={runtimeForm}
            setRuntimeForm={setRuntimeForm}
            setDirtyRuntime={setDirtyRuntime}
            setRawRuntimeToml={setRawRuntimeToml}
          />
        </TabsContent>

        <TabsContent value="browser" className="settings-section">
          <BrowserTab
            loadSection={loadSection}
            browserForm={browserForm}
            setBrowserForm={setBrowserForm}
            setDirtyBrowser={setDirtyBrowser}
            setRawBrowserToml={setRawBrowserToml}
          />
        </TabsContent>

        <TabsContent value="mcp" className="settings-section">
          <McpTab
            mcpServersList={mcpServersList}
            setMcpServersList={setMcpServersList}
            setDirtyMcp={setDirtyMcp}
          />
        </TabsContent>

        <TabsContent value="storage" className="settings-section">
          <StorageTab
            loadSection={loadSection}
            storageForm={storageForm}
            setStorageForm={setStorageForm}
            setDirtyStorage={setDirtyStorage}
            setRawStorageToml={setRawStorageToml}
          />
        </TabsContent>

        <TabsContent value="hooks" className="settings-section">
          <HooksTab
            loadSection={loadSection}
            hooksForm={hooksForm}
            setHooksForm={setHooksForm}
            setDirtyHooks={setDirtyHooks}
            setRawHooksToml={setRawHooksToml}
          />
        </TabsContent>

        <TabsContent value="tools" className="settings-section">
          <ToolsTab
            loadSection={loadSection}
            toolsForm={toolsForm}
            setToolsForm={setToolsForm}
            setDirtyTools={setDirtyTools}
            setRawToolsToml={setRawToolsToml}
          />
        </TabsContent>

        <TabsContent value="guardrails" className="settings-section">
          <GuardrailsTab
            loadSection={loadSection}
            guardrailsForm={guardrailsForm}
            setGuardrailsForm={setGuardrailsForm}
            setDirtyGuardrails={setDirtyGuardrails}
            setRawGuardrailsToml={setRawGuardrailsToml}
          />
        </TabsContent>

        <TabsContent value="knowledge" className="settings-section">
          <KnowledgeTab
            loadSection={loadSection}
            knowledgeForm={knowledgeForm}
            setKnowledgeForm={setKnowledgeForm}
            setDirtyKnowledge={setDirtyKnowledge}
            setRawKnowledgeToml={setRawKnowledgeToml}
          />
        </TabsContent>

        <TabsContent value="prompts">
          <PromptsTab
            setToast={setToast}
            dirtyPrompts={dirtyPrompts}
            setDirtyPrompts={setDirtyPrompts}
          />
        </TabsContent>

        <TabsContent value="about">
          <AboutTab />
        </TabsContent>
      </Tabs>

      {/* Toast notification */}
      {toast && (
        <div className={`settings-toast ${toast.type}`}>
          {toast.message}
        </div>
      )}
    </div>
  );
}
