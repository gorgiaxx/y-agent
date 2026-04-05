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

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ChevronRight,
  ChevronLeft,
  SkipForward,
  Eye,
  EyeOff,
  Info,
  AlertTriangle,
  Settings,
  Zap,
  Search,
} from 'lucide-react';
import { ProviderIconImg } from '../common/ProviderIconPicker';
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

// API types with LobeHub icon IDs, ordered per user request:
// OpenAI-compat first, Anthropic second, OpenAI Response API third
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
// ModelItem & ModelPickerDropdown (like settings ProvidersTab)
// ---------------------------------------------------------------------------

interface ModelItem {
  id: string;
  display_name?: string;
}

function WizardModelPicker({
  models,
  loading,
  error,
  onSelect,
  onClose,
}: {
  models: ModelItem[];
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = useState('');
  const dropdownRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => { inputRef.current?.focus(); }, []);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [onClose]);

  const filtered = models.filter((m) =>
    m.id.toLowerCase().includes(filter.toLowerCase()) ||
    (m.display_name ?? '').toLowerCase().includes(filter.toLowerCase()),
  );

  return (
    <div className="wizard-model-picker" ref={dropdownRef}>
      <div className="wizard-model-picker-search">
        <Search size={12} className="wizard-model-picker-search-icon" />
        <input
          ref={inputRef}
          className="wizard-model-picker-filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter models..."
          onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
        />
      </div>
      {loading && (
        <div className="wizard-model-picker-status">
          <span className="wizard-spinner" /> Fetching models...
        </div>
      )}
      {error && (
        <div className="wizard-model-picker-status wizard-model-picker-error">{error}</div>
      )}
      {!loading && !error && filtered.length === 0 && (
        <div className="wizard-model-picker-status">No models found</div>
      )}
      {!loading && !error && filtered.length > 0 && (
        <div className="wizard-model-picker-list">
          {filtered.map((m) => (
            <button
              key={m.id}
              className="wizard-model-picker-item"
              onClick={() => { onSelect(m.id); onClose(); }}
              type="button"
            >
              <span className="wizard-model-picker-item-id">{m.id}</span>
              {m.display_name && m.display_name !== m.id && (
                <span className="wizard-model-picker-item-name">{m.display_name}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

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
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
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
  const [defaultPermission, setDefaultPermission] = useState(DEFAULT_GUARDRAILS_FORM.default_permission);
  const [hitlAutoApprove, setHitlAutoApprove] = useState(DEFAULT_GUARDRAILS_FORM.hitl_auto_approve_low_risk);
  const [loopGuardMax, setLoopGuardMax] = useState(DEFAULT_GUARDRAILS_FORM.loop_guard_max_iterations);

  // -- Step 5: Knowledge state
  const [knowledgeEnabled, setKnowledgeEnabled] = useState(false);
  const [embeddingModel, setEmbeddingModel] = useState(DEFAULT_KNOWLEDGE_FORM.embedding_model);
  const [embeddingBaseUrl, setEmbeddingBaseUrl] = useState(DEFAULT_KNOWLEDGE_FORM.embedding_base_url);
  const [embeddingApiKeyEnv, setEmbeddingApiKeyEnv] = useState(DEFAULT_KNOWLEDGE_FORM.embedding_api_key_env);

  // Clear test result after a timeout
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
    setModelPickerOpen(true);
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
        At minimum, you need one provider tagged as <strong>general</strong> for the agent to function.
        This will be the primary model used for all tasks.
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

      {/* Main fields */}
      <div className="wizard-row">
        <div className="wizard-field">
          <label className="wizard-field-label">Provider ID</label>
          <input
            className="wizard-input"
            value={provider.id}
            onChange={(e) => updateProvider({ id: e.target.value })}
            placeholder="e.g. my-gpt4"
          />
          <span className="wizard-field-hint">A unique identifier for this provider</span>
        </div>
        <div className="wizard-field">
          <label className="wizard-field-label">Model</label>
          <div className="wizard-model-group">
            <input
              className="wizard-input"
              value={provider.model}
              onChange={(e) => updateProvider({ model: e.target.value })}
              placeholder="e.g. gpt-4o"
            />
            {canDiscoverModels && (
              <button
                className="wizard-model-search-btn"
                onClick={handleModelSearch}
                title="Discover models from endpoint"
                type="button"
              >
                <Search size={13} />
              </button>
            )}
            {modelPickerOpen && (
              <WizardModelPicker
                models={modelList}
                loading={modelLoading}
                error={modelError}
                onSelect={(id) => updateProvider({ model: id })}
                onClose={() => setModelPickerOpen(false)}
              />
            )}
          </div>
        </div>
      </div>

      <div className="wizard-row">
        <div className="wizard-field">
          <label className="wizard-field-label">Base URL</label>
          <input
            className="wizard-input"
            value={provider.base_url ?? ''}
            onChange={(e) => updateProvider({ base_url: e.target.value || null })}
            placeholder={API_TYPE_URLS[provider.provider_type] ?? 'API endpoint URL'}
          />
        </div>
        <div className="wizard-field">
          <label className="wizard-field-label">API Key Env Variable</label>
          <input
            className="wizard-input"
            value={provider.api_key_env ?? ''}
            onChange={(e) => updateProvider({ api_key_env: e.target.value || null })}
            placeholder="e.g. OPENAI_API_KEY"
          />
          <span className="wizard-field-hint">Environment variable name containing the key</span>
        </div>
      </div>

      <div className="wizard-field">
        <label className="wizard-field-label">API Key (direct)</label>
        <div className="wizard-key-group">
          <input
            className="wizard-input wizard-input-password"
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
      </div>

      {/* Test connection */}
      <div className="wizard-test-row">
        <button
          type="button"
          className="wizard-test-btn"
          onClick={handleTestProvider}
          disabled={testing || !provider.model}
        >
          {testing ? <span className="wizard-spinner" /> : <Zap size={12} />}
          {testing ? 'Testing...' : 'Test Connection'}
        </button>
        {testResult && (
          <span className={`wizard-test-result ${testResult.ok ? 'ok' : 'error'}`}>
            {testResult.message}
          </span>
        )}
      </div>

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

      <div className="wizard-toggle-row">
        <div className="wizard-toggle-label">
          <span className="wizard-toggle-label-text">Allow Shell Execution</span>
          <span className="wizard-toggle-label-hint">
            Permit the agent to run shell commands (bash, zsh, etc.) directly on your machine.
            Disable this for a more restricted sandbox environment.
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={allowShell}
          className={`raw-mode-switch-track ${allowShell ? 'raw-mode-switch-track--on' : ''}`}
          onClick={() => setAllowShell(!allowShell)}
        >
          <span className="raw-mode-switch-thumb" />
        </button>
      </div>

      <div className="wizard-info-card">
        <Info size={14} className="wizard-info-card-icon" />
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
        The agent can browse the web using a Chromium-based browser for research and data collection.
      </p>

      <div className="wizard-toggle-row">
        <div className="wizard-toggle-label">
          <span className="wizard-toggle-label-text">Enable Browser</span>
          <span className="wizard-toggle-label-hint">
            Allow the agent to launch and control a browser instance.
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={browserEnabled}
          className={`raw-mode-switch-track ${browserEnabled ? 'raw-mode-switch-track--on' : ''}`}
          onClick={() => setBrowserEnabled(!browserEnabled)}
        >
          <span className="raw-mode-switch-thumb" />
        </button>
      </div>

      {browserEnabled && (
        <>
          <div className="wizard-info-card info">
            <Info size={14} className="wizard-info-card-icon" />
            <span>
              y-agent automatically detects <strong>Chrome</strong>, <strong>Brave</strong>,
              and <strong>Microsoft Edge</strong> browser paths.
              If you use a different Chromium-based browser, specify its path below.
            </span>
          </div>

          <div className="wizard-field">
            <label className="wizard-field-label">Custom Chrome Path (optional)</label>
            <input
              className="wizard-input"
              value={chromePath}
              onChange={(e) => setChromePath(e.target.value)}
              placeholder="Leave empty for auto-detection"
            />
            <span className="wizard-field-hint">
              Only needed for non-standard Chromium browsers (Vivaldi, Arc, etc.)
            </span>
          </div>

          <div className="wizard-toggle-row">
            <div className="wizard-toggle-label">
              <span className="wizard-toggle-label-text">Use System User Profile</span>
              <span className="wizard-toggle-label-hint">
                Use your existing browser profile (cookies, extensions, history).
                When disabled, a clean temporary profile is used each time.
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={useUserProfile}
              className={`raw-mode-switch-track ${useUserProfile ? 'raw-mode-switch-track--on' : ''}`}
              onClick={() => setUseUserProfile(!useUserProfile)}
            >
              <span className="raw-mode-switch-thumb" />
            </button>
          </div>
        </>
      )}
    </div>
  );

  const renderStep3_Guardrails = () => (
    <div key="step-3">
      <h2 className="wizard-step-title">Safety Guardrails</h2>
      <p className="wizard-step-description">
        Configure how the agent handles permissions, human approval, and loop prevention.
      </p>

      <div className="wizard-field">
        <label className="wizard-field-label">Default Permission</label>
        <select
          className="wizard-select"
          value={defaultPermission}
          onChange={(e) => setDefaultPermission(e.target.value)}
        >
          <option value="allow">Allow -- execute without asking</option>
          <option value="notify">Notify -- execute and notify user</option>
          <option value="ask">Ask -- ask for permission before execution</option>
          <option value="deny">Deny -- block execution by default</option>
        </select>
        <span className="wizard-field-hint">
          How tool executions are handled when no specific rule matches.
          &quot;Notify&quot; is recommended for most use cases.
        </span>
      </div>

      <div className="wizard-toggle-row">
        <div className="wizard-toggle-label">
          <span className="wizard-toggle-label-text">HITL Auto-Approve Low Risk</span>
          <span className="wizard-toggle-label-hint">
            Automatically approve tool calls classified as low-risk without
            requiring manual confirmation.
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={hitlAutoApprove}
          className={`raw-mode-switch-track ${hitlAutoApprove ? 'raw-mode-switch-track--on' : ''}`}
          onClick={() => setHitlAutoApprove(!hitlAutoApprove)}
        >
          <span className="raw-mode-switch-thumb" />
        </button>
      </div>

      <div className="wizard-field">
        <label className="wizard-field-label">Loop Guard Max Iterations</label>
        <input
          className="wizard-input-number"
          type="number"
          min={5}
          max={200}
          value={loopGuardMax}
          onChange={(e) => setLoopGuardMax(Number(e.target.value) || 50)}
        />
        <span className="wizard-field-hint">
          Maximum consecutive iterations before the agent is halted to prevent infinite loops.
        </span>
      </div>
    </div>
  );

  const renderStep4_Knowledge = () => (
    <div key="step-4">
      <h2 className="wizard-step-title">Knowledge Base</h2>
      <p className="wizard-step-description">
        The knowledge base enables semantic search over your documents using vector embeddings.
        This requires an embedding API service.
      </p>

      <div className="wizard-toggle-row">
        <div className="wizard-toggle-label">
          <span className="wizard-toggle-label-text">Enable Knowledge Base</span>
          <span className="wizard-toggle-label-hint">
            Requires an embedding model API (e.g., OpenAI text-embedding-3-small).
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={knowledgeEnabled}
          className={`raw-mode-switch-track ${knowledgeEnabled ? 'raw-mode-switch-track--on' : ''}`}
          onClick={() => setKnowledgeEnabled(!knowledgeEnabled)}
        >
          <span className="raw-mode-switch-thumb" />
        </button>
      </div>

      {!knowledgeEnabled && (
        <div className="wizard-info-card">
          <Info size={14} className="wizard-info-card-icon" />
          <span>
            If you are not sure what embedding APIs are, you can safely skip this step.
            The knowledge base can be configured later in Settings.
          </span>
        </div>
      )}

      {knowledgeEnabled && (
        <>
          <div className="wizard-field">
            <label className="wizard-field-label">Embedding Model</label>
            <input
              className="wizard-input"
              value={embeddingModel}
              onChange={(e) => setEmbeddingModel(e.target.value)}
              placeholder="e.g. text-embedding-3-small"
            />
          </div>

          <div className="wizard-field">
            <label className="wizard-field-label">Embedding API Base URL</label>
            <input
              className="wizard-input"
              value={embeddingBaseUrl}
              onChange={(e) => setEmbeddingBaseUrl(e.target.value)}
              placeholder="e.g. https://api.openai.com/v1"
            />
          </div>

          <div className="wizard-field">
            <label className="wizard-field-label">Embedding API Key Env Variable</label>
            <input
              className="wizard-input"
              value={embeddingApiKeyEnv}
              onChange={(e) => setEmbeddingApiKeyEnv(e.target.value)}
              placeholder="e.g. OPENAI_API_KEY"
            />
            <span className="wizard-field-hint">
              Environment variable containing the API key for the embedding service
            </span>
          </div>
        </>
      )}
    </div>
  );

  const renderStep5_Complete = () => (
    <div key="step-5">
      <div className="wizard-complete">
        <div className="wizard-complete-icon">y</div>
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
    <div className="wizard-overlay">
      <div className="wizard-container">
        {/* Header */}
        <div className="wizard-header">
          <div className="wizard-logo">y</div>
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
          {renderCurrentStep()}
        </div>

        {/* Footer */}
        <div className="wizard-footer">
          <div className="wizard-footer-left">
            {step > 0 && !isCompleteStep && (
              <button type="button" className="wizard-btn wizard-btn-back" onClick={handleBack}>
                <ChevronLeft size={14} />
                Back
              </button>
            )}
          </div>
          <div className="wizard-footer-right">
            {!isCompleteStep && (
              <button type="button" className="wizard-btn wizard-btn-skip" onClick={handleSkip}>
                <SkipForward size={14} />
                Skip
              </button>
            )}
            {!isCompleteStep && (
              <button
                type="button"
                className="wizard-btn wizard-btn-next"
                onClick={handleNext}
                disabled={!canProceed() && !skippedSteps.has(step)}
              >
                {isLastConfigStep ? 'Finish Setup' : 'Next'}
                <ChevronRight size={14} />
              </button>
            )}
            {isCompleteStep && (
              <button type="button" className="wizard-btn wizard-btn-finish" onClick={handleFinish}>
                Get Started
                <ChevronRight size={14} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
