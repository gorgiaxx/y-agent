// ---------------------------------------------------------------------------
// SetupWizard -- First-run configuration guide
//
// 6-step wizard that walks users through initial configuration:
//   1. Providers  -- at least one general-purpose provider
//   2. Runtime    -- shell execution toggle
//   3. Browser    -- Chrome path, user profile
//   4. Guardrails -- permission, HITL, loop guard
//   5. Knowledge  -- embedding API
//   6. Complete   -- redirect to settings for more
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ChevronRight,
  ChevronLeft,
  SkipForward,
  Eye,
  EyeOff,
  AlertTriangle,
  Settings,
  Zap,
  Search,
} from 'lucide-react';
import { ProviderIconImg } from '../common/ProviderIconPicker';
import { ModelPickerDropdown, type ModelItem } from '../common/ModelPickerDropdown';
import {
  Input,
  Switch,
  SettingsGroup,
  SettingsItem,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  Button,
} from '../ui';
import '../settings/SettingsForm.css';
import type { GuiConfig } from '../../types';
import {
  type ProviderFormData,
  emptyProvider,
  providersToToml,
  DEFAULT_RUNTIME_FORM,
  DEFAULT_BROWSER_FORM,
  DEFAULT_GUARDRAILS_FORM,
  DEFAULT_KNOWLEDGE_FORM,
} from '../settings/settingsTypes';
import './SetupWizard.css';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TOTAL_STEPS = 6;

const STEP_LABELS = [
  'Providers',
  'Runtime',
  'Browser',
  'Guardrails',
  'Knowledge',
  'Complete',
];

const API_TYPES = [
  { id: 'openai-compat', label: 'OpenAI Compatible', iconId: 'OpenAI' },
  { id: 'anthropic', label: 'Anthropic', iconId: 'Anthropic' },
  { id: 'openai', label: 'OpenAI Response API', iconId: 'OpenAI' },
  { id: 'gemini', label: 'Google Gemini', iconId: 'Gemini' },
  { id: 'deepseek', label: 'DeepSeek', iconId: 'DeepSeek' },
  { id: 'ollama', label: 'Ollama (Local)', iconId: 'Ollama' },
];

const API_TYPE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com/v1',
  gemini: 'https://generativelanguage.googleapis.com/v1beta',
  deepseek: 'https://api.deepseek.com/v1',
  ollama: 'http://localhost:11434/v1',
};


// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface SetupWizardProps {
  config: GuiConfig;
  updateConfig: (updates: Partial<GuiConfig>) => Promise<void>;
  saveSection: (section: string, content: string) => Promise<void>;
  onComplete: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SetupWizard({
  config,
  updateConfig,
  saveSection,
  onComplete,
}: SetupWizardProps) {
  const [step, setStep] = useState(0);
  const [skippedSteps, setSkippedSteps] = useState<Set<number>>(new Set());

  // -- Step 1: Provider state
  const [provider, setProvider] = useState<ProviderFormData>(() => {
    const p = emptyProvider();
    p.provider_type = 'openai-compat';
    p.tags = ['general'];
    return p;
  });
  const [showApiKey, setShowApiKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [useSameForTitle, setUseSameForTitle] = useState<boolean | null>(null);

  // Model discovery state
  const [modelList, setModelList] = useState<ModelItem[]>([]);
  const [modelLoading, setModelLoading] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  // -- Step 2: Runtime state
  const [allowShell, setAllowShell] = useState(DEFAULT_RUNTIME_FORM.allow_shell);

  // -- Step 3: Browser state
  const [browserEnabled, setBrowserEnabled] = useState(DEFAULT_BROWSER_FORM.enabled);
  const [chromePath, setChromePath] = useState('');
  const [useUserProfile, setUseUserProfile] = useState(DEFAULT_BROWSER_FORM.use_user_profile);

  // -- Step 4: Guardrails state
  const [defaultPermission, setDefaultPermission] = useState(
    DEFAULT_GUARDRAILS_FORM.default_permission,
  );
  const [hitlAutoApprove, setHitlAutoApprove] = useState(
    DEFAULT_GUARDRAILS_FORM.hitl_auto_approve_low_risk,
  );
  const [loopGuardMax, setLoopGuardMax] = useState(
    DEFAULT_GUARDRAILS_FORM.loop_guard_max_iterations,
  );

  // -- Step 5: Knowledge state
  const [knowledgeEnabled, setKnowledgeEnabled] = useState(false);
  const [embeddingModel, setEmbeddingModel] = useState(DEFAULT_KNOWLEDGE_FORM.embedding_model);
  const [embeddingBaseUrl, setEmbeddingBaseUrl] = useState(
    DEFAULT_KNOWLEDGE_FORM.embedding_base_url,
  );
  const [embeddingApiKeyEnv, setEmbeddingApiKeyEnv] = useState(
    DEFAULT_KNOWLEDGE_FORM.embedding_api_key_env,
  );

  useEffect(() => {
    if (!testResult) return;
    const t = setTimeout(() => setTestResult(null), 8000);
    return () => clearTimeout(t);
  }, [testResult]);

  // ---- Model discovery ----
  const supportsDiscovery =
    provider.provider_type === 'openai-compat' ||
    provider.provider_type === 'anthropic';
  const canDiscoverModels = supportsDiscovery && !!provider.base_url?.trim();

  const handleModelSearch = useCallback(async () => {
    if (!provider.base_url) return;
    setModelLoading(true);
    setModelError(null);
    setModelList([]);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const result = await invoke<any>('provider_list_models', {
        baseUrl: provider.base_url,
        apiKey: provider.api_key ?? '',
        apiKeyEnv: provider.api_key_env ?? '',
      });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const items: ModelItem[] = (result?.data ?? []).map((m: any) => ({
        id: m.id ?? '',
        display_name: m.display_name ?? m.id ?? '',
      }));
      items.sort((a, b) => a.id.localeCompare(b.id));
      setModelList(items);
    } catch (e) {
      setModelError(String(e));
    } finally {
      setModelLoading(false);
    }
  }, [provider.base_url, provider.api_key, provider.api_key_env]);

  // ---- Handlers ----

  const handleTestProvider = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const msg = await invoke<string>('provider_test', {
        providerType: provider.provider_type,
        model: provider.model,
        apiKey: provider.api_key ?? '',
        apiKeyEnv: provider.api_key_env ?? '',
        baseUrl: provider.base_url ?? null,
      });
      setTestResult({ ok: true, message: msg });
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  }, [provider]);

  const saveProviders = useCallback(async () => {
    const providers: ProviderFormData[] = [provider];
    if (useSameForTitle === true) {
      const titleProvider = {
        ...provider,
        id: `${provider.id}-title`,
        tags: ['title'],
      };
      providers.push(titleProvider);
    }
    const toml = providersToToml(providers);
    await saveSection('providers', toml);
  }, [provider, useSameForTitle, saveSection]);

  const saveRuntime = useCallback(async () => {
    const toml = [
      `default_backend = "${DEFAULT_RUNTIME_FORM.default_backend}"`,
      `allow_shell = ${allowShell}`,
      `allow_host_access = ${DEFAULT_RUNTIME_FORM.allow_host_access}`,
      `default_timeout = "${DEFAULT_RUNTIME_FORM.default_timeout}"`,
      `default_memory_bytes = ${DEFAULT_RUNTIME_FORM.default_memory_bytes}`,
    ].join('\n');
    await saveSection('runtime', toml);
  }, [allowShell, saveSection]);

  const saveBrowser = useCallback(async () => {
    const lines = [
      `enabled = ${browserEnabled}`,
      `launch_mode = "${DEFAULT_BROWSER_FORM.launch_mode}"`,
    ];
    if (chromePath) {
      lines.push(`chrome_path = "${chromePath.replace(/\\/g, '\\\\')}"`);
    }
    lines.push(
      `local_cdp_port = ${DEFAULT_BROWSER_FORM.local_cdp_port}`,
      `use_user_profile = ${useUserProfile}`,
      `cdp_url = "${DEFAULT_BROWSER_FORM.cdp_url}"`,
      `timeout_ms = ${DEFAULT_BROWSER_FORM.timeout_ms}`,
      `allowed_domains = ["*"]`,
      `block_private_networks = ${DEFAULT_BROWSER_FORM.block_private_networks}`,
      `max_screenshot_dim = ${DEFAULT_BROWSER_FORM.max_screenshot_dim}`,
    );
    await saveSection('browser', lines.join('\n'));
  }, [browserEnabled, chromePath, useUserProfile, saveSection]);

  const saveGuardrails = useCallback(async () => {
    const toml = [
      `default_permission = "${defaultPermission}"`,
      `dangerous_auto_ask = ${DEFAULT_GUARDRAILS_FORM.dangerous_auto_ask}`,
      `max_tool_iterations = ${DEFAULT_GUARDRAILS_FORM.max_tool_iterations}`,
      `loop_guard_max_iterations = ${loopGuardMax}`,
      `loop_guard_similarity_threshold = ${DEFAULT_GUARDRAILS_FORM.loop_guard_similarity_threshold}`,
      `risk_high_risk_threshold = ${DEFAULT_GUARDRAILS_FORM.risk_high_risk_threshold}`,
      `hitl_auto_approve_low_risk = ${hitlAutoApprove}`,
    ].join('\n');
    await saveSection('guardrails', toml);
  }, [defaultPermission, hitlAutoApprove, loopGuardMax, saveSection]);

  const saveKnowledge = useCallback(async () => {
    const toml = [
      `l0_max_tokens = ${DEFAULT_KNOWLEDGE_FORM.l0_max_tokens}`,
      `l1_max_tokens = ${DEFAULT_KNOWLEDGE_FORM.l1_max_tokens}`,
      `l2_max_tokens = ${DEFAULT_KNOWLEDGE_FORM.l2_max_tokens}`,
      `max_chunks_per_entry = ${DEFAULT_KNOWLEDGE_FORM.max_chunks_per_entry}`,
      `default_collection = "${DEFAULT_KNOWLEDGE_FORM.default_collection}"`,
      `min_similarity_threshold = ${DEFAULT_KNOWLEDGE_FORM.min_similarity_threshold}`,
      `embedding_enabled = ${knowledgeEnabled}`,
      embeddingModel ? `embedding_model = "${embeddingModel}"` : '',
      embeddingBaseUrl ? `embedding_base_url = "${embeddingBaseUrl}"` : '',
      embeddingApiKeyEnv ? `embedding_api_key_env = "${embeddingApiKeyEnv}"` : '',
      `embedding_dimensions = ${DEFAULT_KNOWLEDGE_FORM.embedding_dimensions}`,
      `retrieval_strategy = "${DEFAULT_KNOWLEDGE_FORM.retrieval_strategy}"`,
      `bm25_weight = ${DEFAULT_KNOWLEDGE_FORM.bm25_weight}`,
      `vector_weight = ${DEFAULT_KNOWLEDGE_FORM.vector_weight}`,
    ].filter(Boolean).join('\n');
    await saveSection('knowledge', toml);
  }, [knowledgeEnabled, embeddingModel, embeddingBaseUrl, embeddingApiKeyEnv, saveSection]);

  const handleNext = useCallback(async () => {
    try {
      if (step === 0 && !skippedSteps.has(0)) await saveProviders();
      if (step === 1 && !skippedSteps.has(1)) await saveRuntime();
      if (step === 2 && !skippedSteps.has(2)) await saveBrowser();
      if (step === 3 && !skippedSteps.has(3)) await saveGuardrails();
      if (step === 4 && !skippedSteps.has(4)) await saveKnowledge();
    } catch (e) {
      console.error('Wizard save error:', e);
    }
    if (step < TOTAL_STEPS - 1) {
      setStep(step + 1);
    }
  }, [step, skippedSteps, saveProviders, saveRuntime, saveBrowser, saveGuardrails, saveKnowledge]);

  const handleSkip = useCallback(() => {
    setSkippedSteps((prev) => new Set(prev).add(step));
    if (step < TOTAL_STEPS - 1) {
      setStep(step + 1);
    }
  }, [step]);

  const handleBack = useCallback(() => {
    if (step > 0) setStep(step - 1);
  }, [step]);

  const handleFinish = useCallback(async () => {
    await updateConfig({ ...config, setup_completed: true });
    try {
      await invoke<string>('config_reload');
    } catch (e) {
      console.error('Reload error:', e);
    }
    onComplete();
  }, [config, updateConfig, onComplete]);

  const updateProvider = (patch: Partial<ProviderFormData>) => {
    setProvider((prev) => ({ ...prev, ...patch }));
  };

  // ---- Step renderers ----

  const renderStep0_Providers = () => (
    <div key="step-0">
      <h2 className="wizard-step-title">Configure Your LLM Provider</h2>
      <p className="wizard-step-description">
        At minimum, you need one provider tagged as <strong>general</strong> for the agent to
        function. This will be the primary model used for all tasks.
      </p>

      {/* API Type selection */}
      <div className="wizard-section">
        <h3 className="wizard-section-title">API Type</h3>
        <div className="wizard-api-types">
          {API_TYPES.map((pt) => (
            <button
              key={pt.id}
              type="button"
              className={`wizard-api-type-btn ${provider.provider_type === pt.id ? 'active' : ''}`}
              onClick={() => {
                updateProvider({
                  provider_type: pt.id,
                  base_url: API_TYPE_URLS[pt.id] ?? provider.base_url,
                });
              }}
            >
              <span className="wizard-api-type-icon">
                <ProviderIconImg iconId={pt.iconId} size={24} />
              </span>
              <span className="wizard-api-type-label">{pt.label}</span>
            </button>
          ))}
        </div>
      </div>

      <SettingsGroup title="Connection">
        <SettingsItem title="Provider ID" wide>
          <Input
            value={provider.id}
            onChange={(e) => updateProvider({ id: e.target.value })}
            placeholder="e.g. my-gpt4"
          />
        </SettingsItem>
        <SettingsItem title="Model" wide>
          <div className="wizard-model-group w-full">
            <Input
              className="flex-1 min-w-0 pr-[30px]"
              value={provider.model}
              onChange={(e) => updateProvider({ model: e.target.value })}
              placeholder="e.g. gpt-4o"
            />
            {canDiscoverModels && (
              <ModelPickerDropdown
                models={modelList}
                loading={modelLoading}
                error={modelError}
                onSelect={(id) => updateProvider({ model: id })}
              >
                <button
                  className="wizard-model-search-btn"
                  onClick={handleModelSearch}
                  title="Discover models from endpoint"
                  type="button"
                >
                  <Search size={13} />
                </button>
              </ModelPickerDropdown>
            )}
          </div>
        </SettingsItem>
        <SettingsItem title="Base URL" wide>
          <Input
            value={provider.base_url ?? ''}
            onChange={(e) => updateProvider({ base_url: e.target.value || null })}
            placeholder={API_TYPE_URLS[provider.provider_type] ?? 'API endpoint URL'}
          />
        </SettingsItem>
        <SettingsItem title="API Key Env Variable" description="Environment variable name containing the key" wide>
          <Input
            value={provider.api_key_env ?? ''}
            onChange={(e) => updateProvider({ api_key_env: e.target.value || null })}
            placeholder="e.g. OPENAI_API_KEY"
          />
        </SettingsItem>
        <SettingsItem title="API Key (direct)" wide>
          <div className="wizard-key-group w-full">
            <Input
              className="flex-1 min-w-0 pr-[30px]"
              variant="mono"
              type={showApiKey ? 'text' : 'password'}
              value={provider.api_key ?? ''}
              onChange={(e) => updateProvider({ api_key: e.target.value || null })}
              placeholder="Direct API key (optional if env var is set)"
            />
            <button
              className="wizard-key-toggle"
              onClick={() => setShowApiKey(!showApiKey)}
              type="button"
              title={showApiKey ? 'Hide' : 'Reveal'}
            >
              {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
        </SettingsItem>
        <SettingsItem title="Test Connection" wide>
          <div className="wizard-test-row">
            <Button
              variant="outline"
              size="sm"
              onClick={handleTestProvider}
              disabled={testing || !provider.model}
            >
              {testing ? <span className="wizard-spinner" /> : <Zap size={12} />}
              {testing ? 'Testing...' : 'Test Connection'}
            </Button>
            {testResult && (
              <span className={`wizard-test-result ${testResult.ok ? 'ok' : 'error'}`}>
                {testResult.message}
              </span>
            )}
          </div>
        </SettingsItem>
      </SettingsGroup>

      {/* Title provider prompt */}
      <div className="wizard-title-prompt">
        <p className="wizard-title-prompt-question">
          Use this provider for generating session titles?
        </p>
        <div className="wizard-info-card warning">
          <AlertTriangle size={14} className="wizard-info-card-icon" />
          <span>
            Title generation consumes tokens on every new conversation.
            A smaller, cheaper model is recommended (e.g. gpt-4o-mini).
          </span>
        </div>
        <div className="wizard-title-prompt-options">
          <button
            type="button"
            className={`wizard-title-prompt-btn ${useSameForTitle === true ? 'selected' : ''}`}
            onClick={() => setUseSameForTitle(true)}
          >
            Yes, use this
          </button>
          <button
            type="button"
            className={`wizard-title-prompt-btn ${useSameForTitle === false ? 'selected' : ''}`}
            onClick={() => setUseSameForTitle(false)}
          >
            No, I will configure separately
          </button>
        </div>
      </div>
    </div>
  );

  const renderStep1_Runtime = () => (
    <div key="step-1">
      <h2 className="wizard-step-title">Runtime Configuration</h2>
      <p className="wizard-step-description">
        Control how the agent executes code and commands on your system.
      </p>

      <SettingsGroup title="Execution">
        <SettingsItem
          title="Allow Shell Execution"
          description="Permit the agent to run shell commands (bash, zsh, etc.) directly on your machine. Disable this for a more restricted sandbox environment."
        >
          <Switch checked={allowShell} onCheckedChange={setAllowShell} />
        </SettingsItem>
      </SettingsGroup>

      <div className="wizard-info-card">
        <Settings size={14} className="wizard-info-card-icon" />
        <span>
          When disabled, the agent will still be able to read and write files,
          but cannot execute arbitrary commands. You can change this later in Settings.
        </span>
      </div>
    </div>
  );

  const renderStep2_Browser = () => (
    <div key="step-2">
      <h2 className="wizard-step-title">Browser Configuration</h2>
      <p className="wizard-step-description">
        The agent can browse the web using a Chromium-based browser for research and data
        collection.
      </p>

      <SettingsGroup title="Browser">
        <SettingsItem
          title="Enable Browser"
          description="Allow the agent to launch and control a browser instance."
        >
          <Switch checked={browserEnabled} onCheckedChange={setBrowserEnabled} />
        </SettingsItem>

        {browserEnabled && (
          <>
            <SettingsItem
              title="Custom Chrome Path"
              description="Only needed for non-standard Chromium browsers (Vivaldi, Arc, etc.)"
              wide
            >
              <Input
                value={chromePath}
                onChange={(e) => setChromePath(e.target.value)}
                placeholder="Leave empty for auto-detection"
              />
            </SettingsItem>
            <SettingsItem
              title="Use System User Profile"
              description="Use your existing browser profile (cookies, extensions, history). When disabled, a clean temporary profile is used each time."
            >
              <Switch checked={useUserProfile} onCheckedChange={setUseUserProfile} />
            </SettingsItem>
          </>
        )}
      </SettingsGroup>

      {browserEnabled && (
        <div className="wizard-info-card info">
          <Settings size={14} className="wizard-info-card-icon" />
          <span>
            y-agent automatically detects <strong>Chrome</strong>, <strong>Brave</strong>,
            and <strong>Microsoft Edge</strong> browser paths.
            If you use a different Chromium-based browser, specify its path above.
          </span>
        </div>
      )}
    </div>
  );

  const renderStep3_Guardrails = () => (
    <div key="step-3">
      <h2 className="wizard-step-title">Safety Guardrails</h2>
      <p className="wizard-step-description">
        Configure how the agent handles permissions, human approval, and loop prevention.
      </p>

      <SettingsGroup title="Permissions">
        <SettingsItem
          title="Default Permission"
          description='How tool executions are handled when no specific rule matches. "Notify" is recommended for most use cases.'
        >
          <Select value={defaultPermission} onValueChange={setDefaultPermission}>
            <SelectTrigger className="w-[200px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="allow">Allow</SelectItem>
              <SelectItem value="notify">Notify</SelectItem>
              <SelectItem value="ask">Ask</SelectItem>
              <SelectItem value="deny">Deny</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem
          title="HITL Auto-Approve Low Risk"
          description="Automatically approve tool calls classified as low-risk without requiring manual confirmation."
        >
          <Switch checked={hitlAutoApprove} onCheckedChange={setHitlAutoApprove} />
        </SettingsItem>
        <SettingsItem
          title="Loop Guard Max Iterations"
          description="Maximum consecutive iterations before the agent is halted to prevent infinite loops."
        >
          <Input
            numeric
            type="number"
            min={5}
            max={200}
            className="w-[100px]"
            value={loopGuardMax}
            onChange={(e) => setLoopGuardMax(Number(e.target.value) || 50)}
          />
        </SettingsItem>
      </SettingsGroup>
    </div>
  );

  const renderStep4_Knowledge = () => (
    <div key="step-4">
      <h2 className="wizard-step-title">Knowledge Base</h2>
      <p className="wizard-step-description">
        The knowledge base enables semantic search over your documents using vector embeddings.
        This requires an embedding API service.
      </p>

      <SettingsGroup title="Embedding">
        <SettingsItem
          title="Enable Knowledge Base"
          description="Requires an embedding model API (e.g., OpenAI text-embedding-3-small)."
        >
          <Switch checked={knowledgeEnabled} onCheckedChange={setKnowledgeEnabled} />
        </SettingsItem>

        {knowledgeEnabled && (
          <>
            <SettingsItem title="Embedding Model" wide>
              <Input
                value={embeddingModel}
                onChange={(e) => setEmbeddingModel(e.target.value)}
                placeholder="e.g. text-embedding-3-small"
              />
            </SettingsItem>
            <SettingsItem title="Embedding API Base URL" wide>
              <Input
                value={embeddingBaseUrl}
                onChange={(e) => setEmbeddingBaseUrl(e.target.value)}
                placeholder="e.g. https://api.openai.com/v1"
              />
            </SettingsItem>
            <SettingsItem
              title="Embedding API Key Env"
              description="Environment variable containing the API key for the embedding service"
              wide
            >
              <Input
                value={embeddingApiKeyEnv}
                onChange={(e) => setEmbeddingApiKeyEnv(e.target.value)}
                placeholder="e.g. OPENAI_API_KEY"
              />
            </SettingsItem>
          </>
        )}
      </SettingsGroup>

      {!knowledgeEnabled && (
        <div className="wizard-info-card">
          <Settings size={14} className="wizard-info-card-icon" />
          <span>
            If you are not sure what embedding APIs are, you can safely skip this step.
            The knowledge base can be configured later in Settings.
          </span>
        </div>
      )}
    </div>
  );

  const renderStep5_Complete = () => (
    <div key="step-5">
      <div className="wizard-complete">
        <img
          src="/logo-256x256.png"
          alt="y-agent"
          className="wizard-complete-logo"
        />
        <h2 className="wizard-complete-title">Setup Complete</h2>
        <p className="wizard-complete-text">
          Your initial configuration is ready. You can start using y-agent right away.
          For more advanced configuration options, visit the Settings panel.
        </p>
        <div className="wizard-complete-settings-hint">
          <Settings size={14} />
          <span>
            Access Settings anytime via the sidebar or the <strong>/settings</strong> command
            to configure MCP servers, sessions, storage, hooks, and more.
          </span>
        </div>
      </div>
    </div>
  );

  // ---- Step renderer dispatch ----
  const renderCurrentStep = () => {
    switch (step) {
      case 0: return renderStep0_Providers();
      case 1: return renderStep1_Runtime();
      case 2: return renderStep2_Browser();
      case 3: return renderStep3_Guardrails();
      case 4: return renderStep4_Knowledge();
      case 5: return renderStep5_Complete();
      default: return null;
    }
  };

  // ---- Timeline step state ----
  const getStepState = (i: number) => {
    if (i < step) {
      return skippedSteps.has(i) ? 'skipped' : 'completed';
    }
    if (i === step) return 'active';
    return 'pending';
  };

  // ---- Can proceed? ----
  const canProceed = () => {
    if (step === 0) {
      return !!(provider.id && provider.model);
    }
    return true;
  };

  const isLastConfigStep = step === TOTAL_STEPS - 2;
  const isCompleteStep = step === TOTAL_STEPS - 1;

  return (
    <div className="wizard-page">
      {/* Header */}
      <div className="wizard-header">
        <img src="/logo-256x256.png" alt="y-agent" className="wizard-logo" />
        <h1 className="wizard-header-title">Setup Wizard</h1>
      </div>

      {/* Timeline step indicator */}
      <div className="wizard-timeline">
        {STEP_LABELS.map((label, i) => (
          <div key={i} className={`wizard-timeline-step ${getStepState(i)}`}>
            <div className="wizard-timeline-step-top">
              <span className="wizard-timeline-dot" />
            </div>
            <span className="wizard-timeline-label">{label}</span>
          </div>
        ))}
      </div>

      {/* Body */}
      <div className="wizard-body" key={step}>
        <div className="wizard-body-inner">
          {renderCurrentStep()}
        </div>
      </div>

      {/* Footer */}
      <div className="wizard-footer">
        <div className="wizard-footer-left">
          {step > 0 && !isCompleteStep && (
            <Button variant="outline" size="sm" onClick={handleBack}>
              <ChevronLeft size={14} />
              Back
            </Button>
          )}
        </div>
        <div className="wizard-footer-right">
          {!isCompleteStep && (
            <Button variant="ghost" size="sm" onClick={handleSkip}>
              <SkipForward size={14} />
              Skip
            </Button>
          )}
          {!isCompleteStep && (
            <Button
              size="sm"
              onClick={handleNext}
              disabled={!canProceed() && !skippedSteps.has(step)}
            >
              {isLastConfigStep ? 'Finish Setup' : 'Next'}
              <ChevronRight size={14} />
            </Button>
          )}
          {isCompleteStep && (
            <Button size="sm" onClick={handleFinish}>
              Get Started
              <ChevronRight size={14} />
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
