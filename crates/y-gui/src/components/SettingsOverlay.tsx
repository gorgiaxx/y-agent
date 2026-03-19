import { useState, useEffect, useCallback, useRef } from 'react';
import { Settings, Plug, Info, X, Eye, EyeOff, RefreshCw, Plus, FileText, RotateCcw } from 'lucide-react';
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

type SettingsTab = 'general' | 'providers' | 'session' | 'runtime' | 'browser' | 'storage' | 'hooks' | 'tools' | 'guardrails' | 'knowledge' | 'prompts' | 'about';

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

interface VolumeMappingData {
  host_path: string;
  container_path: string;
  mode: string;
}

interface RuntimeFormData {
  default_backend: string;
  allow_shell: boolean;
  allow_host_access: boolean;
  default_timeout: string;
  default_memory_bytes: number;
  // SSH fields
  ssh_host: string;
  ssh_port: number;
  ssh_user: string;
  ssh_auth_method: string;
  ssh_password: string;
  ssh_private_key_path: string;
  ssh_passphrase: string;
  ssh_known_hosts_path: string;
  // Docker fields
  docker_default_image: string;
  docker_network_mode: string;
  docker_privileged: boolean;
  docker_user: string;
  docker_readonly_rootfs: boolean;
  docker_default_env: Record<string, string>;
  docker_default_volumes: VolumeMappingData[];
  docker_extra_hosts: string[];
  docker_dns: string[];
  docker_cap_add: string[];
  docker_cap_drop: string[];
  // Python venv (uv) fields
  python_venv_enabled: boolean;
  python_uv_path: string;
  python_version: string;
  python_venv_dir: string;
  python_working_dir: string;
  // Bun venv fields
  bun_venv_enabled: boolean;
  bun_path: string;
  bun_version: string;
  bun_working_dir: string;
}

interface BrowserFormData {
  enabled: boolean;
  auto_launch: boolean;
  headless: boolean;
  chrome_path: string;
  local_cdp_port: number;
  cdp_url: string;
  timeout_ms: number;
  allowed_domains: string[];
  block_private_networks: boolean;
  max_screenshot_dim: number;
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
  const lines: string[] = [
    `default_backend = "${r.default_backend}"`,
    `allow_shell = ${r.allow_shell}`,
    `allow_host_access = ${r.allow_host_access}`,
    `default_timeout = "${r.default_timeout}"`,
    `default_memory_bytes = ${r.default_memory_bytes}`,
    '',
    '[ssh]',
    `host = "${r.ssh_host}"`,
    `port = ${r.ssh_port}`,
    `user = "${r.ssh_user}"`,
    `auth_method = "${r.ssh_auth_method}"`,
  ];
  if (r.ssh_auth_method === 'password' && r.ssh_password) {
    lines.push(`password = "${r.ssh_password}"`);
  }
  if (r.ssh_auth_method === 'public_key' && r.ssh_private_key_path) {
    lines.push(`private_key_path = "${r.ssh_private_key_path}"`);
  }
  if (r.ssh_passphrase) lines.push(`passphrase = "${r.ssh_passphrase}"`);
  if (r.ssh_known_hosts_path) lines.push(`known_hosts_path = "${r.ssh_known_hosts_path}"`);

  lines.push('');
  lines.push('[docker]');
  if (r.docker_default_image) lines.push(`default_image = "${r.docker_default_image}"`);
  lines.push(`network_mode = "${r.docker_network_mode}"`);
  lines.push(`privileged = ${r.docker_privileged}`);
  lines.push(`readonly_rootfs = ${r.docker_readonly_rootfs}`);
  if (r.docker_user) lines.push(`user = "${r.docker_user}"`);  
  if (r.docker_cap_drop.length > 0) {
    lines.push(`cap_drop = [${r.docker_cap_drop.map(c => `"${c}"`).join(', ')}]`);
  }
  if (r.docker_cap_add.length > 0) {
    lines.push(`cap_add = [${r.docker_cap_add.map(c => `"${c}"`).join(', ')}]`);
  }
  if (r.docker_dns.length > 0) {
    lines.push(`dns = [${r.docker_dns.map(d => `"${d}"`).join(', ')}]`);
  }
  if (r.docker_extra_hosts.length > 0) {
    lines.push(`extra_hosts = [${r.docker_extra_hosts.map(h => `"${h}"`).join(', ')}]`);
  }

  // Docker default_env as inline table section.
  if (Object.keys(r.docker_default_env).length > 0) {
    lines.push('');
    lines.push('[docker.default_env]');
    for (const [k, v] of Object.entries(r.docker_default_env)) {
      lines.push(`${k} = "${v}"`);
    }
  }

  // Docker default_volumes as array of tables.
  for (const vol of r.docker_default_volumes) {
    lines.push('');
    lines.push('[[docker.default_volumes]]');
    lines.push(`host_path = "${vol.host_path}"`);
    lines.push(`container_path = "${vol.container_path}"`);
    lines.push(`mode = "${vol.mode}"`);
  }

  // Python venv section.
  lines.push('');
  lines.push('[python_venv]');
  lines.push(`enabled = ${r.python_venv_enabled}`);
  lines.push(`uv_path = "${r.python_uv_path}"`);
  lines.push(`python_version = "${r.python_version}"`);
  lines.push(`venv_dir = "${r.python_venv_dir}"`);
  if (r.python_working_dir) lines.push(`working_dir = "${r.python_working_dir}"`);

  // Bun venv section.
  lines.push('');
  lines.push('[bun_venv]');
  lines.push(`enabled = ${r.bun_venv_enabled}`);
  lines.push(`bun_path = "${r.bun_path}"`);
  lines.push(`bun_version = "${r.bun_version}"`);
  if (r.bun_working_dir) lines.push(`working_dir = "${r.bun_working_dir}"`);

  lines.push('');
  return lines.join('\n');
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
  const ssh = json?.ssh ?? {};
  const docker = json?.docker ?? {};
  const pythonVenv = json?.python_venv ?? {};
  const bunVenv = json?.bun_venv ?? {};
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const volumes: VolumeMappingData[] = (docker.default_volumes ?? []).map((v: any) => ({
    host_path: v.host_path ?? '',
    container_path: v.container_path ?? '',
    mode: v.mode ?? 'ro',
  }));
  return {
    default_backend: json?.default_backend ?? 'native',
    allow_shell: json?.allow_shell ?? false,
    allow_host_access: json?.allow_host_access ?? false,
    default_timeout: json?.default_timeout ?? '30s',
    default_memory_bytes: json?.default_memory_bytes ?? 536870912,
    ssh_host: ssh.host ?? 'localhost',
    ssh_port: ssh.port ?? 22,
    ssh_user: ssh.user ?? 'root',
    ssh_auth_method: ssh.auth_method ?? 'public_key',
    ssh_password: ssh.password ?? '',
    ssh_private_key_path: ssh.private_key_path ?? '',
    ssh_passphrase: ssh.passphrase ?? '',
    ssh_known_hosts_path: ssh.known_hosts_path ?? '',
    docker_default_image: docker.default_image ?? '',
    docker_network_mode: docker.network_mode ?? 'none',
    docker_privileged: docker.privileged ?? false,
    docker_user: docker.user ?? '',
    docker_readonly_rootfs: docker.readonly_rootfs ?? true,
    docker_default_env: docker.default_env ?? {},
    docker_default_volumes: volumes,
    docker_extra_hosts: docker.extra_hosts ?? [],
    docker_dns: docker.dns ?? [],
    docker_cap_add: docker.cap_add ?? [],
    docker_cap_drop: docker.cap_drop ?? ['ALL'],
    python_venv_enabled: pythonVenv.enabled ?? false,
    python_uv_path: pythonVenv.uv_path ?? 'uv',
    python_version: pythonVenv.python_version ?? '3.12',
    python_venv_dir: pythonVenv.venv_dir ?? '.venv',
    python_working_dir: pythonVenv.working_dir ?? '',
    bun_venv_enabled: bunVenv.enabled ?? false,
    bun_path: bunVenv.bun_path ?? 'bun',
    bun_version: bunVenv.bun_version ?? 'latest',
    bun_working_dir: bunVenv.working_dir ?? '',
  };
}


function browserToToml(b: BrowserFormData): string {
  const lines: string[] = [
    `enabled = ${b.enabled}`,
    `auto_launch = ${b.auto_launch}`,
    `headless = ${b.headless}`,
  ];
  if (b.chrome_path) lines.push(`chrome_path = "${b.chrome_path}"`);
  lines.push(`local_cdp_port = ${b.local_cdp_port}`);
  lines.push(`cdp_url = "${b.cdp_url}"`);
  lines.push(`timeout_ms = ${b.timeout_ms}`);
  lines.push(`allowed_domains = [${b.allowed_domains.map(d => `"${d}"`).join(', ')}]`);
  lines.push(`block_private_networks = ${b.block_private_networks}`);
  lines.push(`max_screenshot_dim = ${b.max_screenshot_dim}`);
  return lines.join('\n') + '\n';
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function jsonToBrowser(json: any): BrowserFormData {
  return {
    enabled: json?.enabled ?? true,
    auto_launch: json?.auto_launch ?? false,
    headless: json?.headless ?? true,
    chrome_path: json?.chrome_path ?? '',
    local_cdp_port: json?.local_cdp_port ?? 9222,
    cdp_url: json?.cdp_url ?? 'http://127.0.0.1:9222',
    timeout_ms: json?.timeout_ms ?? 30000,
    allowed_domains: Array.isArray(json?.allowed_domains) ? json.allowed_domains : ['*'],
    block_private_networks: json?.block_private_networks ?? true,
    max_screenshot_dim: json?.max_screenshot_dim ?? 4096,
  };
}

const CONFIG_SECTIONS: { key: SettingsTab; label: string }[] = [
  { key: 'providers', label: 'Providers' },
  { key: 'session', label: 'Session' },
  { key: 'runtime', label: 'Runtime' },
  { key: 'browser', label: 'Browser' },
  { key: 'storage', label: 'Storage' },
  { key: 'hooks', label: 'Hooks' },
  { key: 'tools', label: 'Tools' },
  { key: 'guardrails', label: 'Guardrails' },
  { key: 'knowledge', label: 'Knowledge' },
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
    ssh_host: 'localhost',
    ssh_port: 22,
    ssh_user: 'root',
    ssh_auth_method: 'public_key',
    ssh_password: '',
    ssh_private_key_path: '',
    ssh_passphrase: '',
    ssh_known_hosts_path: '',
    docker_default_image: '',
    docker_network_mode: 'none',
    docker_privileged: false,
    docker_user: '',
    docker_readonly_rootfs: true,
    docker_default_env: {},
    docker_default_volumes: [],
    docker_extra_hosts: [],
    docker_dns: [],
    docker_cap_add: [],
    docker_cap_drop: ['ALL'],
    python_venv_enabled: false,
    python_uv_path: 'uv',
    python_version: '3.12',
    python_venv_dir: '.venv',
    python_working_dir: '',
    bun_venv_enabled: false,
    bun_path: 'bun',
    bun_version: 'latest',
    bun_working_dir: '',
  });
  const [runtimeFormLoading, setRuntimeFormLoading] = useState(false);

  // Structured browser form state.
  const [browserForm, setBrowserForm] = useState<BrowserFormData>({
    enabled: true,
    auto_launch: false,
    headless: true,
    chrome_path: '',
    local_cdp_port: 9222,
    cdp_url: 'http://127.0.0.1:9222',
    timeout_ms: 30000,
    allowed_domains: ['*'],
    block_private_networks: true,
    max_screenshot_dim: 4096,
  });
  const [browserFormLoading, setBrowserFormLoading] = useState(false);

  // Dirty flags -- track which sections have unsaved changes.
  const [dirtyProviders, setDirtyProviders] = useState(false);
  const [dirtySession, setDirtySession] = useState(false);
  const [dirtyRuntime, setDirtyRuntime] = useState(false);
  const [dirtyBrowser, setDirtyBrowser] = useState(false);
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

    if (dirtyBrowser) {
      try {
        await saveSection('browser', browserToToml(browserForm));
        setDirtyBrowser(false);
      } catch (e) { errors.push(`browser: ${e}`); }
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
    onClose();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    dirtyProviders, dirtySession, dirtyRuntime, dirtyBrowser,
    providersList, providersMeta, sessionForm, runtimeForm, browserForm,
    tomlDraftsBySection, dirtyPrompts, saveSection, reloadConfig, localConfig, onSave, onClose,
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

  // Load runtime config as structured JSON via config_get.
  const loadRuntimeForm = useCallback(async () => {
    setRuntimeFormLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const runtimeJson = allConfig?.runtime ?? {};
      setRuntimeForm(jsonToRuntime(runtimeJson));
    } catch {
      // Use defaults if section not found.
    } finally {
      setRuntimeFormLoading(false);
    }
  }, []);

  // Load browser config as structured JSON via config_get.
  const loadBrowserForm = useCallback(async () => {
    setBrowserFormLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const browserJson = allConfig?.browser ?? {};
      setBrowserForm(jsonToBrowser(browserJson));
    } catch {
      // Use defaults if section not found.
    } finally {
      setBrowserFormLoading(false);
    }
  }, []);

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
    } else if (activeTab === 'browser') {
      loadBrowserForm();
    } else if (activeTab === 'prompts') {
      loadPromptFiles();
    } else if (selectedSection) {
      doLoadSection(activeTab);
    }
  }, [activeTab, selectedSection, doLoadSection, loadProviders, loadSessionForm, loadRuntimeForm, loadBrowserForm, loadPromptFiles]);

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
  const isBrowserSection = activeTab === 'browser';
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

  // Handler: restore current prompt to its compiled-in default.
  const handlePromptRestore = useCallback(async () => {
    const filename = promptFiles[activePromptTab];
    if (!filename) return;
    try {
      const defaultContent = await invoke<string>('prompt_get_default', { filename });
      setPromptContent(defaultContent);
      setDirtyPrompts((prev) => ({ ...prev, [filename]: defaultContent }));
      setToast({ message: `Restored "${promptLabel(filename)}" to default`, type: 'success' });
    } catch (e) {
      setToast({ message: `Restore failed: ${e}`, type: 'error' });
    }
  }, [promptFiles, activePromptTab]);

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
              <span className="tab-label">Builtin Prompts</span>
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
                    {!isProviderSection && !isSessionSection && !isRuntimeSection && !isBrowserSection && (
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

                      {/* --- SSH section (shown for all backends since it's config, always saved) --- */}
                      <div style={{ borderTop: '1px solid var(--border)', marginTop: 'var(--space-sm)', paddingTop: 'var(--space-sm)' }}>
                        <h4 style={{ margin: '0 0 var(--space-xs)', fontSize: '0.85rem', color: 'var(--text-secondary)', fontWeight: 600 }}>SSH Configuration</h4>
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">Host</label>
                            <input
                              className="pf-input"
                              value={runtimeForm.ssh_host}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_host: e.target.value }); setDirtyRuntime(true); }}
                              placeholder="localhost"
                            />
                          </div>
                          <div className="pf-field" style={{ maxWidth: '120px' }}>
                            <label className="pf-label">Port</label>
                            <input
                              className="pf-input pf-input-num"
                              type="number"
                              min={1}
                              max={65535}
                              value={runtimeForm.ssh_port}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_port: Number(e.target.value) || 22 }); setDirtyRuntime(true); }}
                            />
                          </div>
                          <div className="pf-field">
                            <label className="pf-label">User</label>
                            <input
                              className="pf-input"
                              value={runtimeForm.ssh_user}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_user: e.target.value }); setDirtyRuntime(true); }}
                              placeholder="root"
                            />
                          </div>
                        </div>
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">Auth Method</label>
                            <select
                              className="form-select"
                              style={{ maxWidth: 'none' }}
                              value={runtimeForm.ssh_auth_method}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_auth_method: e.target.value }); setDirtyRuntime(true); }}
                            >
                              <option value="public_key">Public Key</option>
                              <option value="password">Password</option>
                            </select>
                          </div>
                          {runtimeForm.ssh_auth_method === 'password' ? (
                            <div className="pf-field">
                              <label className="pf-label">Password</label>
                              <input
                                className="pf-input"
                                type="password"
                                value={runtimeForm.ssh_password}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_password: e.target.value }); setDirtyRuntime(true); }}
                                placeholder="SSH password"
                              />
                            </div>
                          ) : (
                            <div className="pf-field">
                              <label className="pf-label">Private Key Path</label>
                              <input
                                className="pf-input"
                                value={runtimeForm.ssh_private_key_path}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_private_key_path: e.target.value }); setDirtyRuntime(true); }}
                                placeholder="~/.ssh/id_rsa"
                              />
                            </div>
                          )}
                        </div>
                        {runtimeForm.ssh_auth_method === 'public_key' && (
                          <div className="pf-row">
                            <div className="pf-field">
                              <label className="pf-label">Passphrase</label>
                              <input
                                className="pf-input"
                                type="password"
                                value={runtimeForm.ssh_passphrase}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_passphrase: e.target.value }); setDirtyRuntime(true); }}
                                placeholder="(optional)"
                              />
                            </div>
                            <div className="pf-field">
                              <label className="pf-label">Known Hosts Path</label>
                              <input
                                className="pf-input"
                                value={runtimeForm.ssh_known_hosts_path}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_known_hosts_path: e.target.value }); setDirtyRuntime(true); }}
                                placeholder="~/.ssh/known_hosts"
                              />
                            </div>
                          </div>
                        )}
                      </div>

                      {/* --- Docker section --- */}
                      <div style={{ borderTop: '1px solid var(--border)', marginTop: 'var(--space-sm)', paddingTop: 'var(--space-sm)' }}>
                        <h4 style={{ margin: '0 0 var(--space-xs)', fontSize: '0.85rem', color: 'var(--text-secondary)', fontWeight: 600 }}>Docker Configuration</h4>
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">Default Image</label>
                            <input
                              className="pf-input"
                              value={runtimeForm.docker_default_image}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_default_image: e.target.value }); setDirtyRuntime(true); }}
                              placeholder="e.g. python:3.12-slim, ubuntu:24.04"
                            />
                            <span className="pf-hint">Container image used for Docker-backend executions when not specified per-request</span>
                          </div>
                        </div>
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">Network Mode</label>
                            <select
                              className="form-select"
                              style={{ maxWidth: 'none' }}
                              value={runtimeForm.docker_network_mode}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_network_mode: e.target.value }); setDirtyRuntime(true); }}
                            >
                              <option value="none">none</option>
                              <option value="bridge">bridge</option>
                              <option value="host">host</option>
                            </select>
                          </div>
                          <div className="pf-field">
                            <label className="pf-label">Container User</label>
                            <input
                              className="pf-input"
                              value={runtimeForm.docker_user}
                              onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_user: e.target.value }); setDirtyRuntime(true); }}
                              placeholder="e.g. 1000:1000"
                            />
                          </div>
                        </div>
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">
                              <input
                                type="checkbox"
                                className="form-checkbox"
                                checked={runtimeForm.docker_privileged}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_privileged: e.target.checked }); setDirtyRuntime(true); }}
                              />
                              {' '}Privileged mode
                            </label>
                          </div>
                          <div className="pf-field">
                            <label className="pf-label">
                              <input
                                type="checkbox"
                                className="form-checkbox"
                                checked={runtimeForm.docker_readonly_rootfs}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_readonly_rootfs: e.target.checked }); setDirtyRuntime(true); }}
                              />
                              {' '}Read-only root filesystem
                            </label>
                          </div>
                        </div>

                        {/* Cap Drop / Cap Add */}
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">Cap Drop</label>
                            <TagChipInput
                              tags={runtimeForm.docker_cap_drop}
                              onChange={(next) => { setRuntimeForm({ ...runtimeForm, docker_cap_drop: next }); setDirtyRuntime(true); }}
                            />
                          </div>
                          <div className="pf-field">
                            <label className="pf-label">Cap Add</label>
                            <TagChipInput
                              tags={runtimeForm.docker_cap_add}
                              onChange={(next) => { setRuntimeForm({ ...runtimeForm, docker_cap_add: next }); setDirtyRuntime(true); }}
                            />
                          </div>
                        </div>

                        {/* DNS / Extra Hosts */}
                        <div className="pf-row">
                          <div className="pf-field">
                            <label className="pf-label">DNS Servers</label>
                            <TagChipInput
                              tags={runtimeForm.docker_dns}
                              onChange={(next) => { setRuntimeForm({ ...runtimeForm, docker_dns: next }); setDirtyRuntime(true); }}
                            />
                          </div>
                          <div className="pf-field">
                            <label className="pf-label">Extra Hosts</label>
                            <TagChipInput
                              tags={runtimeForm.docker_extra_hosts}
                              onChange={(next) => { setRuntimeForm({ ...runtimeForm, docker_extra_hosts: next }); setDirtyRuntime(true); }}
                            />
                          </div>
                        </div>

                        {/* Environment Variables */}
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">Environment Variables</label>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                              {Object.entries(runtimeForm.docker_default_env).map(([k, v], i) => (
                                <div key={i} style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
                                  <input
                                    className="pf-input"
                                    style={{ flex: 1 }}
                                    value={k}
                                    onChange={(e) => {
                                      const entries = Object.entries(runtimeForm.docker_default_env);
                                      entries[i] = [e.target.value, v];
                                      setRuntimeForm({ ...runtimeForm, docker_default_env: Object.fromEntries(entries) });
                                      setDirtyRuntime(true);
                                    }}
                                    placeholder="KEY"
                                  />
                                  <span style={{ color: 'var(--text-secondary)' }}>=</span>
                                  <input
                                    className="pf-input"
                                    style={{ flex: 2 }}
                                    value={v}
                                    onChange={(e) => {
                                      const newEnv = { ...runtimeForm.docker_default_env };
                                      newEnv[k] = e.target.value;
                                      setRuntimeForm({ ...runtimeForm, docker_default_env: newEnv });
                                      setDirtyRuntime(true);
                                    }}
                                    placeholder="value"
                                  />
                                  <button
                                    type="button"
                                    className="pf-tag-chip-remove"
                                    style={{ padding: '2px 6px', cursor: 'pointer' }}
                                    title="Remove"
                                    onClick={() => {
                                      const newEnv = { ...runtimeForm.docker_default_env };
                                      delete newEnv[k];
                                      setRuntimeForm({ ...runtimeForm, docker_default_env: newEnv });
                                      setDirtyRuntime(true);
                                    }}
                                  >×</button>
                                </div>
                              ))}
                              <button
                                type="button"
                                className="btn-test"
                                style={{ alignSelf: 'flex-start', fontSize: '0.75rem', padding: '2px 8px' }}
                                onClick={() => {
                                  const newEnv = { ...runtimeForm.docker_default_env, '': '' };
                                  setRuntimeForm({ ...runtimeForm, docker_default_env: newEnv });
                                  setDirtyRuntime(true);
                                }}
                              >+ Add Variable</button>
                            </div>
                          </div>
                        </div>

                        {/* Volume Mappings */}
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">Volume Mappings</label>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                              {runtimeForm.docker_default_volumes.map((vol, i) => (
                                <div key={i} style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
                                  <input
                                    className="pf-input"
                                    style={{ flex: 2 }}
                                    value={vol.host_path}
                                    onChange={(e) => {
                                      const vols = [...runtimeForm.docker_default_volumes];
                                      vols[i] = { ...vols[i], host_path: e.target.value };
                                      setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                                      setDirtyRuntime(true);
                                    }}
                                    placeholder="Host path"
                                  />
                                  <span style={{ color: 'var(--text-secondary)' }}>→</span>
                                  <input
                                    className="pf-input"
                                    style={{ flex: 2 }}
                                    value={vol.container_path}
                                    onChange={(e) => {
                                      const vols = [...runtimeForm.docker_default_volumes];
                                      vols[i] = { ...vols[i], container_path: e.target.value };
                                      setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                                      setDirtyRuntime(true);
                                    }}
                                    placeholder="Container path"
                                  />
                                  <select
                                    className="form-select"
                                    style={{ width: '70px', minWidth: '70px' }}
                                    value={vol.mode}
                                    onChange={(e) => {
                                      const vols = [...runtimeForm.docker_default_volumes];
                                      vols[i] = { ...vols[i], mode: e.target.value };
                                      setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                                      setDirtyRuntime(true);
                                    }}
                                  >
                                    <option value="ro">ro</option>
                                    <option value="rw">rw</option>
                                  </select>
                                  <button
                                    type="button"
                                    className="pf-tag-chip-remove"
                                    style={{ padding: '2px 6px', cursor: 'pointer' }}
                                    title="Remove"
                                    onClick={() => {
                                      const vols = runtimeForm.docker_default_volumes.filter((_, j) => j !== i);
                                      setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                                      setDirtyRuntime(true);
                                    }}
                                  >×</button>
                                </div>
                              ))}
                              <button
                                type="button"
                                className="btn-test"
                                style={{ alignSelf: 'flex-start', fontSize: '0.75rem', padding: '2px 8px' }}
                                onClick={() => {
                                  const vols = [...runtimeForm.docker_default_volumes, { host_path: '', container_path: '', mode: 'ro' }];
                                  setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                                  setDirtyRuntime(true);
                                }}
                              >+ Add Volume</button>
                            </div>
                          </div>
                        </div>
                      </div>

                      {/* --- Python Environment (uv) section --- */}
                      <div style={{ borderTop: '1px solid var(--border)', marginTop: 'var(--space-sm)', paddingTop: 'var(--space-sm)' }}>
                        <h4 style={{ margin: '0 0 var(--space-xs)', fontSize: '0.85rem', color: 'var(--text-secondary)', fontWeight: 600 }}>Python Environment (uv)</h4>
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">
                              <input
                                type="checkbox"
                                className="form-checkbox"
                                checked={runtimeForm.python_venv_enabled}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_venv_enabled: e.target.checked }); setDirtyRuntime(true); }}
                              />
                              {' '}Enable Python environment
                            </label>
                            <span className="pf-hint">When enabled, the Python venv path is injected into the system prompt so the LLM uses the correct runtime</span>
                          </div>
                        </div>
                        {runtimeForm.python_venv_enabled && (
                          <>
                            <div className="pf-row">
                              <div className="pf-field">
                                <label className="pf-label">uv Binary Path</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.python_uv_path}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_uv_path: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="uv"
                                />
                              </div>
                              <div className="pf-field">
                                <label className="pf-label">Python Version</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.python_version}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_version: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="3.12"
                                />
                              </div>
                            </div>
                            <div className="pf-row">
                              <div className="pf-field">
                                <label className="pf-label">Venv Directory</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.python_venv_dir}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_venv_dir: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder=".venv"
                                />
                              </div>
                              <div className="pf-field">
                                <label className="pf-label">Working Directory</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.python_working_dir}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_working_dir: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="(uses current dir)"
                                />
                              </div>
                            </div>
                          </>
                        )}
                      </div>

                      {/* --- JavaScript Environment (bun) section --- */}
                      <div style={{ borderTop: '1px solid var(--border)', marginTop: 'var(--space-sm)', paddingTop: 'var(--space-sm)' }}>
                        <h4 style={{ margin: '0 0 var(--space-xs)', fontSize: '0.85rem', color: 'var(--text-secondary)', fontWeight: 600 }}>JavaScript Environment (bun)</h4>
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">
                              <input
                                type="checkbox"
                                className="form-checkbox"
                                checked={runtimeForm.bun_venv_enabled}
                                onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_venv_enabled: e.target.checked }); setDirtyRuntime(true); }}
                              />
                              {' '}Enable JavaScript environment
                            </label>
                            <span className="pf-hint">When enabled, the Bun path is injected into the system prompt so the LLM uses the correct JS runtime</span>
                          </div>
                        </div>
                        {runtimeForm.bun_venv_enabled && (
                          <>
                            <div className="pf-row">
                              <div className="pf-field">
                                <label className="pf-label">bun Binary Path</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.bun_path}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_path: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="bun"
                                />
                              </div>
                              <div className="pf-field">
                                <label className="pf-label">Bun Version</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.bun_version}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_version: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="latest"
                                />
                              </div>
                            </div>
                            <div className="pf-row">
                              <div className="pf-field pf-field-full">
                                <label className="pf-label">Working Directory</label>
                                <input
                                  className="pf-input"
                                  value={runtimeForm.bun_working_dir}
                                  onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_working_dir: e.target.value }); setDirtyRuntime(true); }}
                                  placeholder="(uses current dir)"
                                />
                              </div>
                            </div>
                          </>
                        )}
                      </div>
                    </div>
                  )
                ) : isBrowserSection ? (
                  /* Structured browser (CDP) form */
                  browserFormLoading ? (
                    <div className="section-loading">Loading...</div>
                  ) : (
                    <div className="provider-form-wrap">
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
                          <label className="pf-label">
                            <input
                              type="checkbox"
                              className="form-checkbox"
                              checked={browserForm.auto_launch}
                              onChange={(e) => { setBrowserForm({ ...browserForm, auto_launch: e.target.checked }); setDirtyBrowser(true); }}
                            />
                            {' '}Launch local Chrome automatically
                          </label>
                          <span className="pf-hint">When enabled, y-agent spawns a headless Chrome instance. When disabled, connects to a remote CDP endpoint.</span>
                        </div>
                      </div>

                      {browserForm.auto_launch && (
                        <div className="pf-row">
                          <div className="pf-field pf-field-full">
                            <label className="pf-label">
                              <input
                                type="checkbox"
                                className="form-checkbox"
                                checked={browserForm.headless}
                                onChange={(e) => { setBrowserForm({ ...browserForm, headless: e.target.checked }); setDirtyBrowser(true); }}
                              />
                              {' '}Headless mode
                            </label>
                            <span className="pf-hint">Run Chrome without a visible window. Disable for debugging or visual verification.</span>
                          </div>
                        </div>
                      )}

                      {browserForm.auto_launch ? (
                        /* Local Chrome mode */
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
                    Builtin Prompts
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
                        <div className="prompt-editor-actions">
                          <button
                            type="button"
                            className="btn-prompt-restore"
                            onClick={handlePromptRestore}
                            title="Restore to default"
                          >
                            <RotateCcw size={13} />
                            <span>Restore Default</span>
                          </button>
                        </div>
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
                    <span className="about-label">Author</span>
                    <span className="about-value"><a href="https://gorgias.me">Gorgias</a></span>
                  </div>
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
