import { useState, useEffect, useCallback, useRef } from 'react';
import { Settings, Plug, Info, X, Eye, EyeOff, RefreshCw, Plus, FileText } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import type { GuiConfig } from '../types';
import './SettingsOverlay.css';

interface SettingsOverlayProps {
  config: GuiConfig;
  onSave: (updates: Partial<GuiConfig>) => void;
  onClose: () => void;
  loadSection: (section: string) => Promise<string>;
  saveSection: (section: string, content: string) => Promise<void>;
  reloadConfig: () => Promise<string>;
}

type SettingsTab = 'general' | 'providers' | 'session' | 'runtime' | 'storage' | 'hooks' | 'tools' | 'guardrails' | 'prompts' | 'about';

// ---------------------------------------------------------------------------
// Provider form types (mirrors Rust ProviderConfig)
// ---------------------------------------------------------------------------

interface ProviderFormData {
  id: string;
  provider_type: string;
  model: string;
  tags: string[];
  max_concurrency: number;
  context_window: number;
  cost_per_1k_input: number;
  cost_per_1k_output: number;
  api_key: string | null;
  api_key_env: string | null;
  base_url: string | null;
  temperature: number | null;
  top_p: number | null;
  tool_calling_mode: string | null;
}

interface SessionFormData {
  max_depth: number;
  max_active_per_root: number;
  compaction_threshold: number;
  auto_archive_merged: boolean;
}

interface RuntimeFormData {
  default_backend: string;
  allow_shell: boolean;
  allow_host_access: boolean;
  default_timeout: string;
  default_memory_bytes: number;
}

function emptyProvider(): ProviderFormData {
  return {
    id: '',
    provider_type: 'openai',
    model: '',
    tags: [],
    max_concurrency: 5,
    context_window: 128000,
    cost_per_1k_input: 0,
    cost_per_1k_output: 0,
    api_key: null,
    api_key_env: null,
    base_url: null,
    temperature: null,
    top_p: null,
    tool_calling_mode: null,
  };
}

// ---------------------------------------------------------------------------
// TagChipInput -- interactive chip-based tag editor
// ---------------------------------------------------------------------------

function TagChipInput({
  tags,
  onChange,
}: {
  tags: string[];
  onChange: (next: string[]) => void;
}) {
  const [inputValue, setInputValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const addTag = (raw: string) => {
    const trimmed = raw.trim().replace(/,$/, '');
    if (trimmed && !tags.includes(trimmed)) {
      onChange([...tags, trimmed]);
    }
    setInputValue('');
  };

  const removeTag = (index: number) => {
    onChange(tags.filter((_, i) => i !== index));
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addTag(inputValue);
    } else if (e.key === 'Backspace' && inputValue === '' && tags.length > 0) {
      onChange(tags.slice(0, -1));
    }
  };

  const handleBlur = () => {
    if (inputValue.trim()) {
      addTag(inputValue);
    }
  };

  return (
    <div className="pf-tag-input-wrap" onClick={() => inputRef.current?.focus()}>
      {tags.map((tag, i) => (
        <span key={i} className="pf-tag-chip">
          <span className="pf-tag-chip-text">{tag}</span>
          <button
            type="button"
            className="pf-tag-chip-remove"
            onClick={(e) => { e.stopPropagation(); removeTag(i); }}
            title={`Remove tag "${tag}"`}
          >
            x
          </button>
        </span>
      ))}
      <input
        ref={inputRef}
        className="pf-tag-text-input"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        placeholder={tags.length === 0 ? 'Add tags (Enter or comma to confirm)' : ''}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProviderTabPanel -- flat form for a single provider (shown in tab view)
// ---------------------------------------------------------------------------

function ProviderTabPanel({
  provider,
  index,
  onChange,
}: {
  provider: ProviderFormData;
  index: number;
  onChange: (index: number, updated: ProviderFormData) => void;
}) {
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  const update = (patch: Partial<ProviderFormData>) => {
    onChange(index, { ...provider, ...patch });
  };

  // Clear test result after 8 seconds.
  useEffect(() => {
    if (!testResult) return;
    const t = setTimeout(() => setTestResult(null), 8000);
    return () => clearTimeout(t);
  }, [testResult]);

  // Also clear test result when provider changes.
  useEffect(() => {
    setTestResult(null);
  }, [provider.id]);

  const handleTest = async () => {
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
  };

  return (
    <div className="provider-tab-form">
      {/* Row 1: ID + Type */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">ID</label>
          <input
            className="pf-input"
            value={provider.id}
            onChange={(e) => update({ id: e.target.value })}
            placeholder="e.g. openai-gpt4"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Provider Type</label>
          <select
            className="form-select"
            style={{ maxWidth: 'none' }}
            value={provider.provider_type}
            onChange={(e) => update({ provider_type: e.target.value })}
          >
            <option value="openai">OpenAI (native API)</option>
            <option value="openai-compat">OpenAI-compatible (vLLM, LiteLLM...)</option>
            <option value="anthropic">Anthropic</option>
            <option value="gemini">Gemini</option>
            <option value="deepseek">DeepSeek</option>
            <option value="ollama">Ollama</option>
            <option value="azure">Azure OpenAI</option>
          </select>
        </div>
      </div>

      {/* Row 2: Model + Base URL */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Model</label>
          <input
            className="pf-input"
            value={provider.model}
            onChange={(e) => update({ model: e.target.value })}
            placeholder="e.g. gpt-4o"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Base URL</label>
          <input
            className="pf-input"
            value={provider.base_url ?? ''}
            onChange={(e) => update({ base_url: e.target.value || null })}
            placeholder="Default"
          />
        </div>
      </div>

      {/* Row 3: API Key + API Key Env */}
      <div className="pf-row">
        <div className="pf-field pf-field-key">
          <label className="pf-label">API Key</label>
          <div className="pf-key-group">
            <input
              className="pf-input"
              type={showKey ? 'text' : 'password'}
              value={provider.api_key ?? ''}
              onChange={(e) => update({ api_key: e.target.value || null })}
              placeholder="Direct key (optional)"
            />
            <button
              className="pf-key-toggle"
              onClick={() => setShowKey(!showKey)}
              title={showKey ? 'Hide' : 'Reveal'}
              type="button"
            >
              {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
        </div>
        <div className="pf-field">
          <label className="pf-label">API Key Env Var</label>
          <input
            className="pf-input"
            value={provider.api_key_env ?? ''}
            onChange={(e) => update({ api_key_env: e.target.value || null })}
            placeholder="e.g. OPENAI_API_KEY"
          />
        </div>
      </div>

      {/* Row 4: Tags -- chip editor */}
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Tags</label>
          <TagChipInput
            tags={provider.tags}
            onChange={(next) => update({ tags: next })}
          />
          <span className="pf-hint">Routing tags -- press Enter or comma to confirm each tag</span>
        </div>
      </div>

      {/* Row 5: Numeric settings */}
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Max Concurrency</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            value={provider.max_concurrency}
            onChange={(e) => update({ max_concurrency: Number(e.target.value) || 1 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Context Window</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            value={provider.context_window}
            onChange={(e) => update({ context_window: Number(e.target.value) || 128000 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Temperature</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.1}
            min={0}
            max={2}
            value={provider.temperature ?? ''}
            onChange={(e) => update({ temperature: e.target.value ? Number(e.target.value) : null })}
            placeholder="--"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Top P</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.05}
            min={0}
            max={1}
            value={provider.top_p ?? ''}
            onChange={(e) => update({ top_p: e.target.value ? Number(e.target.value) : null })}
            placeholder="--"
          />
        </div>
      </div>

      {/* Row 6: Cost */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Cost / 1k Input Tokens</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.0001}
            min={0}
            value={provider.cost_per_1k_input}
            onChange={(e) => update({ cost_per_1k_input: Number(e.target.value) || 0 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Cost / 1k Output Tokens</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.0001}
            min={0}
            value={provider.cost_per_1k_output}
            onChange={(e) => update({ cost_per_1k_output: Number(e.target.value) || 0 })}
          />
        </div>
      </div>

      {/* Test connection row */}
      <div className="pf-row" style={{ borderTop: '1px solid var(--border)', paddingTop: 'var(--space-sm)', marginTop: 'var(--space-xs)' }}>
        <div className="pf-field pf-field-full" style={{ flexDirection: 'row', alignItems: 'center', gap: 'var(--space-sm)' }}>
          <button
            type="button"
            className="btn-test"
            onClick={handleTest}
            disabled={testing}
          >
            {testing ? <span className="pf-spinner" /> : null}
            {testing ? 'Testing...' : 'Test Connection'}
          </button>
          {testResult && (
            <span className={`pf-test-result ${testResult.ok ? 'ok' : 'error'}`}>
              {testResult.message}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Mask sensitive values in TOML content. */
function maskSensitive(content: string): string {
  return content.replace(
    /^(\s*(?:api_key|password|secret|token)\s*=\s*)"([^"]+)"/gm,
    (_match, prefix, value) => `${prefix}"${'*'.repeat(Math.min(value.length, 24))}"`,
  );
}

/** Convert ProviderFormData[] to TOML string. */
function providersToToml(providers: ProviderFormData[]): string {
  const lines: string[] = [];
  for (const p of providers) {
    lines.push('[[providers]]');
    lines.push(`id = "${p.id}"`);
    lines.push(`provider_type = "${p.provider_type}"`);
    lines.push(`model = "${p.model}"`);
    if (p.tags.length > 0) {
      lines.push(`tags = [${p.tags.map((t) => `"${t}"`).join(', ')}]`);
    }
    lines.push(`max_concurrency = ${p.max_concurrency}`);
    lines.push(`context_window = ${p.context_window}`);
    if (p.cost_per_1k_input > 0) lines.push(`cost_per_1k_input = ${p.cost_per_1k_input}`);
    if (p.cost_per_1k_output > 0) lines.push(`cost_per_1k_output = ${p.cost_per_1k_output}`);
    if (p.api_key) lines.push(`api_key = "${p.api_key}"`);
    if (p.api_key_env) lines.push(`api_key_env = "${p.api_key_env}"`);
    if (p.base_url) lines.push(`base_url = "${p.base_url}"`);
    if (p.temperature !== null) lines.push(`temperature = ${p.temperature}`);
    if (p.top_p !== null) lines.push(`top_p = ${p.top_p}`);
    if (p.tool_calling_mode) lines.push(`tool_calling_mode = "${p.tool_calling_mode}"`);
    lines.push('');
  }
  return lines.join('\n');
}

function sessionToToml(s: SessionFormData): string {
  return [
    `max_depth = ${s.max_depth}`,
    `max_active_per_root = ${s.max_active_per_root}`,
    `compaction_threshold = ${s.compaction_threshold}`,
    `auto_archive_merged = ${s.auto_archive_merged}`,
  ].join('\n') + '\n';
}

function runtimeToToml(r: RuntimeFormData): string {
  return [
    `default_backend = "${r.default_backend}"`,
    `allow_shell = ${r.allow_shell}`,
    `allow_host_access = ${r.allow_host_access}`,
    `default_timeout = "${r.default_timeout}"`,
    `default_memory_bytes = ${r.default_memory_bytes}`,
  ].join('\n') + '\n';
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function jsonToProviders(json: any): ProviderFormData[] {
  // config_get nests each section's parsed TOML under the section name.
  // providers.toml parses to { providers: [...], ...meta }, then gets stored as
  // merged["providers"], so the actual array lives at json.providers.providers.
  // Fall back to json.providers directly for forward-compatibility.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let list: any = null;
  if (Array.isArray(json?.providers)) {
    list = json.providers;
  } else if (Array.isArray(json?.providers?.providers)) {
    list = json.providers.providers;
  }
  if (!list) return [];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return list.map((p: any) => ({
    id: p.id ?? '',
    provider_type: p.provider_type ?? 'openai',
    model: p.model ?? '',
    tags: Array.isArray(p.tags) ? p.tags : [],
    max_concurrency: p.max_concurrency ?? 5,
    context_window: p.context_window ?? 128000,
    cost_per_1k_input: p.cost_per_1k_input ?? 0,
    cost_per_1k_output: p.cost_per_1k_output ?? 0,
    api_key: p.api_key ?? null,
    api_key_env: p.api_key_env ?? null,
    base_url: p.base_url ?? null,
    temperature: p.temperature ?? null,
    top_p: p.top_p ?? null,
    tool_calling_mode: p.tool_calling_mode ?? null,
  }));
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function jsonToSession(json: any): SessionFormData {
  return {
    max_depth: json?.max_depth ?? 16,
    max_active_per_root: json?.max_active_per_root ?? 8,
    compaction_threshold: json?.compaction_threshold ?? 80000,
    auto_archive_merged: json?.auto_archive_merged ?? true,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function jsonToRuntime(json: any): RuntimeFormData {
  return {
    default_backend: json?.default_backend ?? 'native',
    allow_shell: json?.allow_shell ?? false,
    allow_host_access: json?.allow_host_access ?? false,
    default_timeout: json?.default_timeout ?? '30s',
    default_memory_bytes: json?.default_memory_bytes ?? 536870912,
  };
}


const CONFIG_SECTIONS: { key: SettingsTab; label: string }[] = [
  { key: 'providers', label: 'Providers' },
  { key: 'session', label: 'Session' },
  { key: 'runtime', label: 'Runtime' },
  { key: 'storage', label: 'Storage' },
  { key: 'hooks', label: 'Hooks' },
  { key: 'tools', label: 'Tools' },
  { key: 'guardrails', label: 'Guardrails' },
];

export function SettingsOverlay({
  config,
  onSave,
  onClose,
  loadSection,
  saveSection,
  reloadConfig,
}: SettingsOverlayProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>('general');
  const [localConfig, setLocalConfig] = useState<GuiConfig>({ ...config });

  // Provider editor state (selectedSection derived from activeTab for TOML sections).
  const [sectionContent, setSectionContent] = useState('');
  const [sectionLoading, setSectionLoading] = useState(false);
  const [showSensitive, setShowSensitive] = useState(false);
  const [toast, setToast] = useState<{ message: string; type: 'success' | 'error' } | null>(null);
  const [rawContent, setRawContent] = useState('');

  // Structured provider form state.
  const [providersList, setProvidersList] = useState<ProviderFormData[]>([]);
  const [providersLoading, setProvidersLoading] = useState(false);
  // Pool-level TOML lines (everything before the first [[providers]] table)
  // cached at load time so they are not lost on save.
  const [providersMeta, setProvidersMeta] = useState('');
  // Index of the active provider tab.
  const [activeProviderTab, setActiveProviderTab] = useState(0);

  // Structured session form state.
  const [sessionForm, setSessionForm] = useState<SessionFormData>({
    max_depth: 16,
    max_active_per_root: 8,
    compaction_threshold: 80000,
    auto_archive_merged: true,
  });
  const [sessionFormLoading, setSessionFormLoading] = useState(false);

  // Structured runtime form state.
  const [runtimeForm, setRuntimeForm] = useState<RuntimeFormData>({
    default_backend: 'native',
    allow_shell: false,
    allow_host_access: false,
    default_timeout: '30s',
    default_memory_bytes: 536870912,
  });
  const [runtimeFormLoading, setRuntimeFormLoading] = useState(false);

  // Dirty flags -- track which sections have unsaved changes.
  const [dirtyProviders, setDirtyProviders] = useState(false);
  const [dirtySession, setDirtySession] = useState(false);
  const [dirtyRuntime, setDirtyRuntime] = useState(false);
  // Per-section raw TOML drafts for the raw-editor sections (hooks, tools, etc.).
  const [tomlDraftsBySection, setTomlDraftsBySection] = useState<Record<string, string>>({});
  // Whether the unified Save Changes is currently writing.
  const [saving, setSaving] = useState(false);

  // Prompt editor state.
  const [promptFiles, setPromptFiles] = useState<string[]>([]);
  const [activePromptTab, setActivePromptTab] = useState(0);
  const [promptContent, setPromptContent] = useState('');
  const [promptLoading, setPromptLoading] = useState(false);
  const [dirtyPrompts, setDirtyPrompts] = useState<Record<string, string>>({});

  // Unified save -- flush all pending changes to disk, then propagate GUI config.
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
        await saveSection('session', sessionToToml(sessionForm));
        setDirtySession(false);
      } catch (e) { errors.push(`session: ${e}`); }
    }

    if (dirtyRuntime) {
      try {
        await saveSection('runtime', runtimeToToml(runtimeForm));
        setDirtyRuntime(false);
      } catch (e) { errors.push(`runtime: ${e}`); }
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

    onSave(localConfig);
    onClose();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    dirtyProviders, dirtySession, dirtyRuntime,
    providersList, providersMeta, sessionForm, runtimeForm,
    tomlDraftsBySection, dirtyPrompts, saveSection, localConfig, onSave, onClose,
  ]);

  // Derive the active config section key from the active tab.
  const selectedSection = (activeTab !== 'general' && activeTab !== 'about' && activeTab !== 'prompts') ? activeTab : null;

  // Load section content when switching sections (for non-providers TOML editor).
  const doLoadSection = useCallback(
    async (section: string) => {
      setSectionLoading(true);
      try {
        const content = await loadSection(section);
        setRawContent(content);
        setSectionContent(showSensitive ? content : maskSensitive(content));
      } catch (e) {
        setToast({ message: `Failed to load: ${e}`, type: 'error' });
      } finally {
        setSectionLoading(false);
      }
    },
    [loadSection, showSensitive],
  );

  // Load providers as structured JSON.
  const loadProviders = useCallback(async () => {
    setProvidersLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      setProvidersList(jsonToProviders(allConfig));

      // Also read raw TOML to cache pool-level meta fields (lines before the
      // first [[providers]] table, e.g. default_freeze_duration_secs).
      try {
        const raw: string = await loadSection('providers');
        const firstTable = raw.indexOf('[[providers]]');
        setProvidersMeta(firstTable > 0 ? raw.slice(0, firstTable) : '');
      } catch {
        setProvidersMeta('');
      }
    } catch (e) {
      setToast({ message: `Failed to load providers: ${e}`, type: 'error' });
    } finally {
      setProvidersLoading(false);
    }
  }, [loadSection]);

  // Load session config as structured JSON.
  const loadSessionForm = useCallback(async () => {
    setSessionFormLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const content = await loadSection('session');
      // Parse TOML-like key=value pairs into an object for the form.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const parsed: any = {};
      for (const line of content.split('\n')) {
        const m = line.match(/^\s*(\w+)\s*=\s*(.+)$/);
        if (m) {
          const val = m[2].trim();
          if (val === 'true') parsed[m[1]] = true;
          else if (val === 'false') parsed[m[1]] = false;
          else if (/^\d+$/.test(val)) parsed[m[1]] = parseInt(val, 10);
          else parsed[m[1]] = val.replace(/^"|"$/g, '');
        }
      }
      setSessionForm(jsonToSession(parsed));
    } catch {
      // Use defaults if section not found.
    } finally {
      setSessionFormLoading(false);
    }
  }, [loadSection]);

  // Load runtime config as structured JSON.
  const loadRuntimeForm = useCallback(async () => {
    setRuntimeFormLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const content = await loadSection('runtime');
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const parsed: any = {};
      for (const line of content.split('\n')) {
        const m = line.match(/^\s*(\w+)\s*=\s*(.+)$/);
        if (m) {
          const val = m[2].trim();
          if (val === 'true') parsed[m[1]] = true;
          else if (val === 'false') parsed[m[1]] = false;
          else if (/^\d+$/.test(val)) parsed[m[1]] = parseInt(val, 10);
          else parsed[m[1]] = val.replace(/^"|"$/g, '');
        }
      }
      setRuntimeForm(jsonToRuntime(parsed));
    } catch {
      // Use defaults if section not found.
    } finally {
      setRuntimeFormLoading(false);
    }
  }, [loadSection]);

  // Load prompt file list and first file content when switching to prompts tab.
  const loadPromptFile = useCallback(async (filename: string) => {
    setPromptLoading(true);
    try {
      const content = await invoke<string>('prompt_get', { filename });
      setPromptContent(content);
    } catch (e) {
      setToast({ message: `Failed to load prompt: ${e}`, type: 'error' });
    } finally {
      setPromptLoading(false);
    }
  }, []);

  const loadPromptFiles = useCallback(async () => {
    setPromptLoading(true);
    try {
      const files = await invoke<string[]>('prompt_list');
      setPromptFiles(files);
      setActivePromptTab(0);
      if (files.length > 0) {
        // Check if there's a dirty draft for this file.
        const firstFile = files[0];
        if (dirtyPrompts[firstFile] !== undefined) {
          setPromptContent(dirtyPrompts[firstFile]);
          setPromptLoading(false);
        } else {
          await loadPromptFile(firstFile);
        }
      } else {
        setPromptContent('');
        setPromptLoading(false);
      }
    } catch (e) {
      setToast({ message: `Failed to list prompts: ${e}`, type: 'error' });
      setPromptLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadPromptFile]);

  useEffect(() => {
    if (activeTab === 'providers') {
      loadProviders();
    } else if (activeTab === 'session') {
      loadSessionForm();
    } else if (activeTab === 'runtime') {
      loadRuntimeForm();
    } else if (activeTab === 'prompts') {
      loadPromptFiles();
    } else if (selectedSection) {
      doLoadSection(activeTab);
    }
  }, [activeTab, selectedSection, doLoadSection, loadProviders, loadSessionForm, loadRuntimeForm, loadPromptFiles]);

  // Toggle sensitive field visibility.
  const handleToggleSensitive = useCallback(() => {
    const next = !showSensitive;
    setShowSensitive(next);
    if (next) {
      // Reveal: show actual content.
      setSectionContent(rawContent);
    } else {
      // Mask: apply masking.
      setSectionContent(maskSensitive(rawContent));
    }
  }, [showSensitive, rawContent]);

  // (All section saves now go through the unified handleSave.
  //  The reload handler is kept for the Reload button in the config header.)


  // Hot-reload config.
  const handleReload = useCallback(async () => {
    try {
      const msg = await reloadConfig();
      setToast({ message: msg, type: 'success' });
    } catch (e) {
      setToast({ message: `Reload failed: ${e}`, type: 'error' });
    }
  }, [reloadConfig]);

  // Provider form handlers.
  const handleProviderChange = useCallback((index: number, updated: ProviderFormData) => {
    setProvidersList((prev) => prev.map((p, i) => (i === index ? updated : p)));
    setDirtyProviders(true);
  }, []);

  const handleProviderRemove = useCallback((index: number) => {
    setProvidersList((prev) => {
      const next = prev.filter((_, i) => i !== index);
      return next;
    });
    setActiveProviderTab((prev) => Math.max(0, prev > index ? prev - 1 : Math.min(prev, providersList.length - 2)));
    setDirtyProviders(true);
  }, [providersList.length]);

  const handleProviderAdd = useCallback(() => {
    setProvidersList((prev) => {
      setActiveProviderTab(prev.length);
      return [...prev, emptyProvider()];
    });
    setDirtyProviders(true);
  }, []);

  // Auto-dismiss toast.
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(timer);
  }, [toast]);

  // Is the section one with a structured form?
  const isProviderSection = activeTab === 'providers';
  const isSessionSection = activeTab === 'session';
  const isRuntimeSection = activeTab === 'runtime';
  const isPromptsSection = activeTab === 'prompts';

  // Handler: switch prompt sub-tab.
  const handlePromptTabSwitch = useCallback(async (index: number) => {
    setActivePromptTab(index);
    const filename = promptFiles[index];
    if (!filename) return;
    // Use dirty draft if available, otherwise load from disk.
    if (dirtyPrompts[filename] !== undefined) {
      setPromptContent(dirtyPrompts[filename]);
    } else {
      await loadPromptFile(filename);
    }
  }, [promptFiles, dirtyPrompts, loadPromptFile]);

  // Friendly label: strip "core_" prefix and ".txt" suffix.
  const promptLabel = (filename: string) =>
    filename.replace(/^core_/, '').replace(/\.txt$/, '');

  return (
    <div className="settings-backdrop" onClick={onClose}>
      <div className="settings-overlay" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <h2 className="settings-title">Settings</h2>
          <button className="btn-close" onClick={onClose}><X size={16} /></button>
        </div>

        <div className="settings-body">
          <nav className="settings-tabs">
            <button
              className={`settings-tab ${activeTab === 'general' ? 'active' : ''}`}
              onClick={() => setActiveTab('general')}
            >
              <span className="tab-icon"><Settings size={14} /></span>
              <span className="tab-label">General</span>
            </button>
            <div className="settings-tab-group-label">Config</div>
            {CONFIG_SECTIONS.map((s) => (
              <button
                key={s.key}
                className={`settings-tab ${activeTab === s.key ? 'active' : ''}`}
                onClick={() => setActiveTab(s.key)}
              >
                <span className="tab-icon"><Plug size={14} /></span>
                <span className="tab-label">{s.label}</span>
              </button>
            ))}
            <div className="settings-tab-group-label">Prompts</div>
            <button
              className={`settings-tab ${activeTab === 'prompts' ? 'active' : ''}`}
              onClick={() => setActiveTab('prompts')}
            >
              <span className="tab-icon"><FileText size={14} /></span>
              <span className="tab-label">Prompts</span>
            </button>
            <div className="settings-tab-separator" />
            <button
              className={`settings-tab ${activeTab === 'about' ? 'active' : ''}`}
              onClick={() => setActiveTab('about')}
            >
              <span className="tab-icon"><Info size={14} /></span>
              <span className="tab-label">About</span>
            </button>
          </nav>

          <div className="settings-content">
            {activeTab === 'general' && (
              <div className="settings-section">
                <h3 className="section-title">Appearance</h3>

                <div className="form-group">
                  <label className="form-label">Theme</label>
                  <select
                    className="form-select"
                    value={localConfig.theme}
                    onChange={(e) =>
                      setLocalConfig({ ...localConfig, theme: e.target.value as GuiConfig['theme'] })
                    }
                  >
                    <option value="dark">Dark</option>
                    <option value="light">Light</option>
                    <option value="system">System</option>
                  </select>
                </div>

                <div className="form-group">
                  <label className="form-label">Font Size</label>
                  <div className="form-range-group">
                    <input
                      type="range"
                      className="form-range"
                      min="12"
                      max="20"
                      value={localConfig.font_size}
                      onChange={(e) =>
                        setLocalConfig({ ...localConfig, font_size: Number(e.target.value) })
                      }
                    />
                    <span className="range-value">{localConfig.font_size}px</span>
                  </div>
                </div>

                <h3 className="section-title">Behavior</h3>

                <div className="form-group">
                  <label className="form-label">
                    <input
                      type="checkbox"
                      className="form-checkbox"
                      checked={localConfig.send_on_enter}
                      onChange={(e) =>
                        setLocalConfig({ ...localConfig, send_on_enter: e.target.checked })
                      }
                    />
                    Send message on Enter
                  </label>
                  <p className="form-hint">
                    When enabled, press Enter to send and Shift+Enter for newline.
                  </p>
                </div>
              </div>
            )}

            {selectedSection && (
              <div className="settings-section">
                <div className="provider-header">
                  <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                    {CONFIG_SECTIONS.find((s) => s.key === activeTab)?.label ?? activeTab}
                  </h3>
                  <div className="provider-actions">
                    {!isProviderSection && !isSessionSection && !isRuntimeSection && (
                      <button
                        className="btn-provider-action"
                        onClick={handleToggleSensitive}
                        title={showSensitive ? 'Hide sensitive values' : 'Show sensitive values'}
                      >
                        {showSensitive ? <EyeOff size={14} /> : <Eye size={14} />}
                      </button>
                    )}
                    <button
                      className="btn-provider-action btn-reload"
                      onClick={handleReload}
                      title="Hot-reload configuration"
                    >
                      <RefreshCw size={14} />
                      <span>Reload</span>
                    </button>
                  </div>
                </div>

                {isProviderSection ? (
                  /* Tabbed provider form: one tab per provider */
                  providersLoading ? (
                    <div className="section-loading">Loading...</div>
                  ) : (
                    <div className="provider-form-wrap">
                      {/* Sub-tab bar */}
                      <div className="provider-subtabs">
                        {providersList.map((p, i) => (
                          <button
                            key={i}
                            className={`provider-subtab ${activeProviderTab === i ? 'active' : ''}`}
                            onClick={() => setActiveProviderTab(i)}
                          >
                            <span className="provider-subtab-label">{p.id || `Provider ${i + 1}`}</span>
                            <span
                              className="provider-subtab-close"
                              role="button"
                              tabIndex={0}
                              title="Remove provider"
                              onClick={(e) => { e.stopPropagation(); handleProviderRemove(i); }}
                              onKeyDown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleProviderRemove(i); } }}
                            >
                              <X size={11} />
                            </span>
                          </button>
                        ))}
                        <button
                          className="provider-subtab provider-subtab-add"
                          onClick={handleProviderAdd}
                          title="Add provider"
                        >
                          <Plus size={13} />
                        </button>
                      </div>

                      {/* Active provider form */}
                      {providersList.length === 0 ? (
                        <div className="provider-empty">
                          No providers configured. Click + to add one.
                        </div>
                      ) : (
                        <ProviderTabPanel
                          key={activeProviderTab}
                          provider={providersList[activeProviderTab] ?? providersList[0]}
                          index={activeProviderTab < providersList.length ? activeProviderTab : 0}
                          onChange={handleProviderChange}
                        />
                      )}
                    </div>
                  )
                ) : isSessionSection ? (
                  /* Structured session form */
                  sessionFormLoading ? (
                    <div className="section-loading">Loading...</div>
                  ) : (
                    <div className="provider-form-wrap">
                      <div className="pf-row pf-row-quad">
                        <div className="pf-field">
                          <label className="pf-label">Max Tree Depth</label>
                          <input
                            className="pf-input pf-input-num"
                            type="number"
                            min={1}
                            value={sessionForm.max_depth}
                            onChange={(e) => { setSessionForm({ ...sessionForm, max_depth: Number(e.target.value) || 16 }); setDirtySession(true); }}
                          />
                        </div>
                        <div className="pf-field">
                          <label className="pf-label">Max Active per Root</label>
                          <input
                            className="pf-input pf-input-num"
                            type="number"
                            min={1}
                            value={sessionForm.max_active_per_root}
                            onChange={(e) => { setSessionForm({ ...sessionForm, max_active_per_root: Number(e.target.value) || 8 }); setDirtySession(true); }}
                          />
                        </div>
                        <div className="pf-field">
                          <label className="pf-label">Compaction Threshold (tokens)</label>
                          <input
                            className="pf-input pf-input-num"
                            type="number"
                            min={1000}
                            step={1000}
                            value={sessionForm.compaction_threshold}
                            onChange={(e) => { setSessionForm({ ...sessionForm, compaction_threshold: Number(e.target.value) || 80000 }); setDirtySession(true); }}
                          />
                        </div>
                      </div>
                      <div className="pf-row">
                        <div className="pf-field pf-field-full">
                          <label className="pf-label">
                            <input
                              type="checkbox"
                              className="form-checkbox"
                              checked={sessionForm.auto_archive_merged}
                              onChange={(e) => { setSessionForm({ ...sessionForm, auto_archive_merged: e.target.checked }); setDirtySession(true); }}
                            />
                            {' '}Auto-archive merged sessions
                          </label>
                        </div>
                      </div>
                    </div>
                  )
                ) : isRuntimeSection ? (
                  /* Structured runtime form */
                  runtimeFormLoading ? (
                    <div className="section-loading">Loading...</div>
                  ) : (
                    <div className="provider-form-wrap">
                      <div className="pf-row">
                        <div className="pf-field">
                          <label className="pf-label">Default Backend</label>
                          <select
                            className="form-select"
                            style={{ maxWidth: 'none' }}
                            value={runtimeForm.default_backend}
                            onChange={(e) => { setRuntimeForm({ ...runtimeForm, default_backend: e.target.value }); setDirtyRuntime(true); }}
                          >
                            <option value="native">Native</option>
                            <option value="docker">Docker</option>
                            <option value="ssh">SSH</option>
                          </select>
                        </div>
                        <div className="pf-field">
                          <label className="pf-label">Default Timeout</label>
                          <input
                            className="pf-input"
                            value={runtimeForm.default_timeout}
                            onChange={(e) => { setRuntimeForm({ ...runtimeForm, default_timeout: e.target.value }); setDirtyRuntime(true); }}
                            placeholder="e.g. 30s, 5m"
                          />
                        </div>
                      </div>
                      <div className="pf-row">
                        <div className="pf-field">
                          <label className="pf-label">Memory Limit (bytes)</label>
                          <input
                            className="pf-input pf-input-num"
                            type="number"
                            min={0}
                            step={1048576}
                            value={runtimeForm.default_memory_bytes}
                            onChange={(e) => { setRuntimeForm({ ...runtimeForm, default_memory_bytes: Number(e.target.value) || 536870912 }); setDirtyRuntime(true); }}
                          />
                        </div>
                      </div>
                      <div className="pf-row">
                        <div className="pf-field pf-field-full">
                          <label className="pf-label">
                            <input
                              type="checkbox"
                              className="form-checkbox"
                              checked={runtimeForm.allow_shell}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, allow_shell: e.target.checked }); setDirtyRuntime(true); }}
                            />
                            {' '}Allow shell execution
                          </label>
                        </div>
                      </div>
                      <div className="pf-row">
                        <div className="pf-field pf-field-full">
                          <label className="pf-label">
                            <input
                              type="checkbox"
                              className="form-checkbox"
                              checked={runtimeForm.allow_host_access}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, allow_host_access: e.target.checked }); setDirtyRuntime(true); }}
                            />
                            {' '}Allow host filesystem access
                          </label>
                        </div>
                      </div>
                    </div>
                  )
                ) : (
                  /* TOML editor for other sections */
                  sectionLoading ? (
                    <div className="section-loading">Loading...</div>
                  ) : (
                    <div className="toml-editor-wrap">
                      <textarea
                        className="toml-editor"
                        value={sectionContent}
                        onChange={(e) => {
                          const val = e.target.value;
                          setSectionContent(val);
                          setRawContent(val);
                          setTomlDraftsBySection((prev) => ({ ...prev, [activeTab]: val }));
                        }}
                        spellCheck={false}
                        placeholder={`No ${activeTab}.toml found. Content will be created on save.`}
                      />
                      {!showSensitive && sectionContent && (
                        <p className="form-hint">
                          Sensitive values are masked. Click the eye icon to reveal them.
                        </p>
                      )}
                    </div>
                  )
                )}

              </div>
            )}

            {isPromptsSection && (
              <div className="settings-section">
                <div className="provider-header">
                  <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
                    Prompts
                  </h3>
                </div>
                {promptLoading && promptFiles.length === 0 ? (
                  <div className="section-loading">Loading...</div>
                ) : promptFiles.length === 0 ? (
                  <div className="provider-empty">
                    No prompt files found. Run&nbsp;<code>y-agent init</code>&nbsp;to seed defaults.
                  </div>
                ) : (
                  <div className="provider-form-wrap">
                    {/* Sub-tab bar — one tab per prompt file, no add/close */}
                    <div className="provider-subtabs">
                      {promptFiles.map((f, i) => (
                        <button
                          key={f}
                          className={`provider-subtab ${activePromptTab === i ? 'active' : ''}`}
                          onClick={() => handlePromptTabSwitch(i)}
                        >
                          <span className="provider-subtab-label">{promptLabel(f)}</span>
                        </button>
                      ))}
                    </div>

                    {/* Prompt content textarea */}
                    {promptLoading ? (
                      <div className="section-loading">Loading...</div>
                    ) : (
                      <div className="toml-editor-wrap">
                        <textarea
                          className="toml-editor prompt-editor"
                          value={promptContent}
                          onChange={(e) => {
                            const val = e.target.value;
                            setPromptContent(val);
                            const filename = promptFiles[activePromptTab];
                            if (filename) {
                              setDirtyPrompts((prev) => ({ ...prev, [filename]: val }));
                            }
                          }}
                          spellCheck={false}
                          placeholder="Empty prompt. Type content here."
                        />
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {activeTab === 'about' && (
              <div className="settings-section">
                <h3 className="section-title">y-agent Desktop</h3>
                <div className="about-info">
                  <div className="about-row">
                    <span className="about-label">Version</span>
                    <span className="about-value">0.1.0</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Framework</span>
                    <span className="about-value">Tauri v2</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Frontend</span>
                    <span className="about-value">React + TypeScript</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Backend</span>
                    <span className="about-value">Rust (y-service)</span>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Toast notification */}
        {toast && (
          <div className={`settings-toast ${toast.type}`}>
            {toast.message}
          </div>
        )}

        <div className="settings-footer">
          <button className="btn-cancel" onClick={onClose} disabled={saving}>Cancel</button>
          <button className="btn-save" onClick={handleSave} disabled={saving}>
            {saving ? 'Saving...' : 'Save Changes'}
          </button>
        </div>
      </div>
    </div>
  );
}
