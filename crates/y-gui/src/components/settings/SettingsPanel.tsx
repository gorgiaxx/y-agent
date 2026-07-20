// ---------------------------------------------------------------------------
// SettingsPanel -- Orchestrator that delegates to tab-specific components.
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { transport } from '../../lib';
import type { AppConfigResponse, GuiConfig } from '../../types';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import {
  SESSION_SCHEMA, BROWSER_SCHEMA, RUNTIME_SCHEMA,
  BACKGROUND_WAKE_SCHEMA, LSP_SCHEMA,
  STORAGE_SCHEMA, HOOKS_SCHEMA, TOOLS_SCHEMA, GUARDRAILS_SCHEMA, KNOWLEDGE_SCHEMA,
  LANGFUSE_SCHEMA,
} from '../../utils/settingsSchemas';

import type {
  ProviderFormData,
  SessionFormData,
  BackgroundWakeFormData,
  LspFormData,
  RuntimeFormData,
  BrowserFormData,
  McpServerFormData,
  StorageFormData,
  HooksFormData,
  ToolsFormData,
  GuardrailsFormData,
  KnowledgeFormData,
  LangfuseFormData,
} from './settingsTypes';
import {
  DEFAULT_SESSION_FORM,
  DEFAULT_BACKGROUND_WAKE_FORM,
  DEFAULT_LSP_FORM,
  DEFAULT_RUNTIME_FORM,
  DEFAULT_BROWSER_FORM,
  DEFAULT_STORAGE_FORM,
  DEFAULT_HOOKS_FORM,
  DEFAULT_TOOLS_FORM,
  DEFAULT_GUARDRAILS_FORM,
  DEFAULT_KNOWLEDGE_FORM,
  DEFAULT_LANGFUSE_FORM,
  jsonToProviders,
  buildProvidersToml,
  jsonToRetry,
  stripRetrySection,
  RETRY_DEFAULTS,
  type RetryFormData,
  mcpServersToJson,
  TAB_LABELS,
} from './settingsTypes';

import { GeneralTab } from './GeneralTab';
import { ProvidersTab } from './ProvidersTab';
import { SessionTab } from './SessionTab';
import { BackgroundWakeTab } from './BackgroundWakeTab';
import { RuntimeTab } from './RuntimeTab';
import { LspTab } from './LspTab';
import { CapabilityPacksTab } from './CapabilityPacksTab';
import { BrowserTab } from './BrowserTab';
import { McpTab } from './McpTab';
import { StorageTab } from './StorageTab';
import { HooksTab } from './HooksTab';
import { ToolsTab } from './ToolsTab';
import { GuardrailsTab } from './GuardrailsTab';
import { KnowledgeTab } from './KnowledgeTab';
import { LangfuseTab } from './LangfuseTab';
import { PromptsTab } from './PromptsTab';
import { PromptTemplatesTab } from './PromptTemplatesTab';
import { AboutTab } from './AboutTab';
import { SettingsActionSlotProvider, SettingsActionSlotTarget } from './TomlEditorTab';
import { Button, Tabs, TabsContent, WindowControls } from '../ui';

import './SettingsPanel.css';
import './SettingsForm.css';
import { useRuntimeCapabilities } from '../../hooks/useRuntimeCapabilities';

type PromptTemplateSaveHandler = () => Promise<void>;

interface SettingsPanelProps {
  config: GuiConfig;
  activeTab: SettingsTab;
  onSave: (updates: Partial<GuiConfig>) => void;
  loadSection: (section: string) => Promise<string>;
  saveSection: (section: string, content: string) => Promise<void>;
  reloadConfig: () => Promise<string>;
  onRunWizard?: () => void;
}

export type SettingsTab = 'general' | 'providers' | 'session' | 'backgroundWake' | 'runtime' | 'lsp' | 'capabilityPacks' | 'browser' | 'mcp' | 'storage' | 'hooks' | 'tools' | 'guardrails' | 'knowledge' | 'langfuse' | 'promptTemplates' | 'prompts' | 'about';

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
  const {
    capabilities: runtimeCapabilities,
    error: runtimeCapabilitiesError,
  } = useRuntimeCapabilities();

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
  const [retryForm, setRetryForm] = useState<RetryFormData>(RETRY_DEFAULTS);
  const [dirtySession, setDirtySession] = useState(false);
  const [rawSessionToml, setRawSessionToml] = useState<string | undefined>(undefined);

  // Background auto-wake
  const [backgroundWakeForm, setBackgroundWakeForm] = useState<BackgroundWakeFormData>({ ...DEFAULT_BACKGROUND_WAKE_FORM });
  const [dirtyBackgroundWake, setDirtyBackgroundWake] = useState(false);
  const [rawBackgroundWakeToml, setRawBackgroundWakeToml] = useState<string | undefined>(undefined);

  // Runtime
  const [runtimeForm, setRuntimeForm] = useState<RuntimeFormData>({ ...DEFAULT_RUNTIME_FORM });
  const [dirtyRuntime, setDirtyRuntime] = useState(false);
  const [rawRuntimeToml, setRawRuntimeToml] = useState<string | undefined>(undefined);

  // Language servers
  const [lspForm, setLspForm] = useState<LspFormData>({
    ...DEFAULT_LSP_FORM,
    servers: DEFAULT_LSP_FORM.servers.map((server) => ({ ...server })),
  });
  const [dirtyLsp, setDirtyLsp] = useState(false);
  const [rawLspToml, setRawLspToml] = useState<string | undefined>(undefined);

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

  // Langfuse
  const [langfuseForm, setLangfuseForm] = useState<LangfuseFormData>({ ...DEFAULT_LANGFUSE_FORM });
  const [dirtyLangfuse, setDirtyLangfuse] = useState(false);
  const [rawLangfuseToml, setRawLangfuseToml] = useState<string | undefined>(undefined);

  // Prompts
  const [dirtyPrompts, setDirtyPrompts] = useState<Record<string, string>>({});

  // Prompt templates
  const [dirtyPromptTemplates, setDirtyPromptTemplates] = useState(false);
  const [promptTemplateSaveHandler, setPromptTemplateSaveHandler] =
    useState<PromptTemplateSaveHandler | null>(null);

  const registerPromptTemplateSaveHandler = useCallback((handler: PromptTemplateSaveHandler | null) => {
    setPromptTemplateSaveHandler(() => handler);
  }, []);

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
        const toml = rawProvidersToml !== undefined
          ? rawProvidersToml
          : buildProvidersToml(providersMeta, retryForm, providersList);
        await saveSection('providers', toml);
        setRawProvidersToml(undefined);
        setDirtyProviders(false);
        // Refresh form-mode state from the freshly persisted TOML so that
        // switching from raw mode back to form mode reflects the saved data.
        try {
          const allConfig = await transport.invoke<AppConfigResponse>('config_get');
          setProvidersList(jsonToProviders(allConfig));
          setRetryForm(jsonToRetry(allConfig));
          const raw = await loadSection('providers');
          const tableMatch = raw.match(/^\s*\[\[providers\]\]/m);
          const firstTable = tableMatch?.index ?? -1;
          setProvidersMeta(stripRetrySection(firstTable > 0 ? raw.slice(0, firstTable) : ''));
        } catch (refreshErr) {
          console.warn('Providers refresh after save failed:', refreshErr);
        }
      } catch (e) { errors.push(`providers: ${e}`); }
    }

    // Canonical TOML-backed sections: merge form into raw TOML, save, clear dirty.
    const sectionRegistry = [
      { section: 'session', dirty: dirtySession, setDirty: setDirtySession, raw: rawSessionToml, setRaw: setRawSessionToml, form: sessionForm, schema: SESSION_SCHEMA },
      { section: 'background_auto_wake', dirty: dirtyBackgroundWake, setDirty: setDirtyBackgroundWake, raw: rawBackgroundWakeToml, setRaw: setRawBackgroundWakeToml, form: backgroundWakeForm, schema: BACKGROUND_WAKE_SCHEMA },
      { section: 'runtime', dirty: dirtyRuntime, setDirty: setDirtyRuntime, raw: rawRuntimeToml, setRaw: setRawRuntimeToml, form: runtimeForm, schema: RUNTIME_SCHEMA },
      { section: 'lsp', dirty: dirtyLsp, setDirty: setDirtyLsp, raw: rawLspToml, setRaw: setRawLspToml, form: lspForm, schema: LSP_SCHEMA },
      { section: 'browser', dirty: dirtyBrowser, setDirty: setDirtyBrowser, raw: rawBrowserToml, setRaw: setRawBrowserToml, form: browserForm, schema: BROWSER_SCHEMA },
      { section: 'storage', dirty: dirtyStorage, setDirty: setDirtyStorage, raw: rawStorageToml, setRaw: setRawStorageToml, form: storageForm, schema: STORAGE_SCHEMA },
      { section: 'hooks', dirty: dirtyHooks, setDirty: setDirtyHooks, raw: rawHooksToml, setRaw: setRawHooksToml, form: hooksForm, schema: HOOKS_SCHEMA },
      { section: 'tools', dirty: dirtyTools, setDirty: setDirtyTools, raw: rawToolsToml, setRaw: setRawToolsToml, form: toolsForm, schema: TOOLS_SCHEMA },
      { section: 'guardrails', dirty: dirtyGuardrails, setDirty: setDirtyGuardrails, raw: rawGuardrailsToml, setRaw: setRawGuardrailsToml, form: guardrailsForm, schema: GUARDRAILS_SCHEMA },
      { section: 'knowledge', dirty: dirtyKnowledge, setDirty: setDirtyKnowledge, raw: rawKnowledgeToml, setRaw: setRawKnowledgeToml, form: knowledgeForm, schema: KNOWLEDGE_SCHEMA },
      { section: 'langfuse', dirty: dirtyLangfuse, setDirty: setDirtyLangfuse, raw: rawLangfuseToml, setRaw: setRawLangfuseToml, form: langfuseForm, schema: LANGFUSE_SCHEMA },
    ] as const;

    for (const s of sectionRegistry) {
      if (!s.dirty) continue;
      try {
        const toml = mergeIntoRawToml(s.raw, s.form as unknown as Record<string, unknown>, s.schema);
        await saveSection(s.section, toml);
        s.setRaw(toml);
        s.setDirty(false);
      } catch (e) { errors.push(`${s.section}: ${e}`); }
    }

    if (dirtyMcp) {
      try {
        const json = mcpServersToJson(mcpServersList);
        await transport.invoke('mcp_config_save', { content: json });
        setDirtyMcp(false);
      } catch (e) { errors.push(`mcp: ${e}`); }
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

    if (dirtyPromptTemplates) {
      try {
        if (!promptTemplateSaveHandler) {
          throw new Error('Prompt template editor is not ready');
        }
        await promptTemplateSaveHandler();
        setDirtyPromptTemplates(false);
      } catch (e) {
        errors.push(`prompt templates: ${e}`);
      }
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
    dirtyProviders, dirtySession, dirtyBackgroundWake, dirtyRuntime, dirtyLsp, dirtyBrowser, dirtyMcp,
    dirtyStorage, dirtyHooks, dirtyTools, dirtyGuardrails, dirtyKnowledge, dirtyLangfuse,
    providersList, providersMeta, retryForm, sessionForm, backgroundWakeForm, runtimeForm, lspForm, browserForm, mcpServersList,
    storageForm, hooksForm, toolsForm, guardrailsForm, knowledgeForm, langfuseForm,
    rawSessionToml, rawBackgroundWakeToml, rawRuntimeToml, rawLspToml, rawBrowserToml, rawProvidersToml,
    rawStorageToml, rawHooksToml, rawToolsToml, rawGuardrailsToml, rawKnowledgeToml, rawLangfuseToml,
    dirtyPrompts, dirtyPromptTemplates, promptTemplateSaveHandler,
    saveSection, loadSection, reloadConfig, localConfig, onSave,
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
      <SettingsActionSlotProvider>
        <div className="settings-action-bar" data-tauri-drag-region>
          <h2 className="settings-action-bar-title">{TAB_LABELS[activeTab] ?? activeTab}</h2>
          <div className="settings-action-bar-actions">
            <SettingsActionSlotTarget className="settings-action-bar-toggle-slot" />
            <Button variant="primary" onClick={handleSave} disabled={saving}>
              {saving ? 'Saving...' : 'Save Changes'}
            </Button>
            <WindowControls />
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
            retryForm={retryForm}
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
            retryForm={retryForm}
            setRetryForm={setRetryForm}
            setDirtyProviders={setDirtyProviders}
            compactionPrefireAvailability={runtimeCapabilities?.compaction_prefire}
            compactionPrefireAvailabilityError={runtimeCapabilitiesError}
          />
        </TabsContent>

        <TabsContent value="backgroundWake" className="settings-section">
          <BackgroundWakeTab
            loadSection={loadSection}
            form={backgroundWakeForm}
            setForm={setBackgroundWakeForm}
            setDirty={setDirtyBackgroundWake}
            setRawToml={setRawBackgroundWakeToml}
            availability={runtimeCapabilities?.background_auto_wake}
            availabilityError={runtimeCapabilitiesError}
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

        <TabsContent value="lsp" className="settings-section">
          <LspTab
            loadSection={loadSection}
            form={lspForm}
            setForm={setLspForm}
            setDirty={setDirtyLsp}
            setRawToml={setRawLspToml}
            availability={runtimeCapabilities?.lsp}
            availabilityError={runtimeCapabilitiesError}
          />
        </TabsContent>

        <TabsContent value="capabilityPacks" className="settings-section">
          <CapabilityPacksTab
            availability={runtimeCapabilities?.capability_packs}
            availabilityError={runtimeCapabilitiesError}
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
            handlerAvailability={runtimeCapabilities?.hook_handlers}
            llmHookAvailability={runtimeCapabilities?.llm_hooks}
            availabilityError={runtimeCapabilitiesError}
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

        <TabsContent value="langfuse" className="settings-section">
          <LangfuseTab
            loadSection={loadSection}
            langfuseForm={langfuseForm}
            setLangfuseForm={setLangfuseForm}
            setDirtyLangfuse={setDirtyLangfuse}
            setRawLangfuseToml={setRawLangfuseToml}
          />
        </TabsContent>

        <TabsContent value="promptTemplates">
          <PromptTemplatesTab
            setToast={setToast}
            setDirtyPromptTemplates={setDirtyPromptTemplates}
            registerSaveHandler={registerPromptTemplateSaveHandler}
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
      </SettingsActionSlotProvider>

      {/* Toast notification */}
      {toast && (
        <div className={`settings-toast ${toast.type}`}>
          {toast.message}
        </div>
      )}
    </div>
  );
}
