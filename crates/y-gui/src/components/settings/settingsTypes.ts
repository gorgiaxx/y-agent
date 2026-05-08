// ---------------------------------------------------------------------------
// Shared types and helpers for the Settings panel tab components.
// ---------------------------------------------------------------------------

import type { SettingsTab } from './SettingsPanel';
import { escapeTomlString, deserializeFromJson, mergeIntoRawToml } from '../../utils/tomlUtils';
import {
  SESSION_SCHEMA, BROWSER_SCHEMA, RUNTIME_SCHEMA, browserPostProcess,
  STORAGE_SCHEMA, HOOKS_SCHEMA, TOOLS_SCHEMA, GUARDRAILS_SCHEMA, KNOWLEDGE_SCHEMA,
} from '../../utils/settingsSchemas';

// ---------------------------------------------------------------------------
// Provider form types (mirrors Rust ProviderConfig)
// ---------------------------------------------------------------------------

export interface ProviderFormData {
  id: string;
  provider_type: string;
  model: string;
  enabled: boolean;
  tags: string[];
  capabilities: string[];
  max_concurrency: number;
  context_window: number;
  cost_per_1k_input: number;
  cost_per_1k_output: number;
  api_key: string | null;
  api_key_env: string | null;
  base_url: string | null;
  headers: Record<string, string>;
  http_protocol: 'http1' | 'http2';
  temperature: number | null;
  top_p: number | null;
  tool_calling_mode: string | null;
  icon: string | null;
}

export interface SessionFormData {
  max_depth: number;
  max_active_per_root: number;
  compaction_threshold_pct: number;
  auto_archive_merged: boolean;
  // Pruning fields (nested [pruning] section in session.toml)
  pruning_enabled: boolean;
  pruning_token_threshold: number;
  pruning_strategy: string;
  pruning_progressive_max_retries: number;
  pruning_progressive_preserve_identifiers: boolean;
}

export interface VolumeMappingData {
  host_path: string;
  container_path: string;
  mode: string;
}

export interface RuntimeFormData {
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

export interface BrowserFormData {
  enabled: boolean;
  launch_mode: 'remote' | 'auto_launch_headless' | 'auto_launch_visible';
  chrome_path: string;
  local_cdp_port: number;
  use_user_profile: boolean;
  cdp_url: string;
  timeout_ms: number;
  allowed_domains: string[];
  block_private_networks: boolean;
  max_screenshot_dim: number;
}

export interface StorageFormData {
  db_path: string;
  pool_size: number;
  wal_enabled: boolean;
  busy_timeout_ms: number;
  transcript_dir: string;
}

export interface HooksFormData {
  middleware_timeout_ms: number;
  event_channel_capacity: number;
  max_subscribers: number;
}

export interface ToolsFormData {
  max_active: number;
  search_limit: number;
  allow_dynamic_tools: boolean;
}

export interface GuardrailsFormData {
  default_permission: string;
  dangerous_auto_ask: boolean;
  max_tool_iterations: number;
  loop_guard_max_iterations: number;
  loop_guard_similarity_threshold: number;
  risk_high_risk_threshold: number;
  hitl_auto_approve_low_risk: boolean;
}

export interface KnowledgeFormData {
  l0_max_tokens: number;
  l1_max_tokens: number;
  l2_max_tokens: number;
  max_chunks_per_entry: number;
  default_collection: string;
  min_similarity_threshold: number;
  embedding_enabled: boolean;
  embedding_model: string;
  embedding_dimensions: number;
  embedding_base_url: string;
  embedding_api_key_env: string;
  embedding_api_key: string;
  embedding_max_tokens: number;
  retrieval_strategy: string;
  bm25_weight: number;
  vector_weight: number;
}

export interface McpServerFormData {
  name: string;
  transport: 'stdio' | 'http';
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string;
  headers: Record<string, string>;
  bearer_token: string;
  cwd: string;
  startup_timeout_secs: number;
  tool_timeout_secs: number;
  alwaysAllow: string[];
  disabled: boolean;
}

// ---------------------------------------------------------------------------
// Factory functions
// ---------------------------------------------------------------------------

export function emptyProvider(): ProviderFormData {
  return {
    id: '',
    provider_type: 'openai',
    model: '',
    enabled: true,
    tags: [],
    capabilities: [],
    max_concurrency: 5,
    context_window: 128000,
    cost_per_1k_input: 0,
    cost_per_1k_output: 0,
    api_key: null,
    api_key_env: null,
    base_url: null,
    headers: {},
    http_protocol: 'http1',
    temperature: null,
    top_p: null,
    tool_calling_mode: null,
    icon: null,
  };
}

export function emptyMcpServer(): McpServerFormData {
  return {
    name: '',
    transport: 'stdio',
    command: '',
    args: [],
    env: {},
    url: '',
    headers: {},
    bearer_token: '',
    cwd: '',
    startup_timeout_secs: 30,
    tool_timeout_secs: 120,
    alwaysAllow: [],
    disabled: false,
  };
}

export const DEFAULT_SESSION_FORM: SessionFormData = {
  max_depth: 16,
  max_active_per_root: 8,
  compaction_threshold_pct: 85,
  auto_archive_merged: true,
  pruning_enabled: true,
  pruning_token_threshold: 2000,
  pruning_strategy: 'auto',
  pruning_progressive_max_retries: 2,
  pruning_progressive_preserve_identifiers: true,
};

export const DEFAULT_RUNTIME_FORM: RuntimeFormData = {
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
};

export const DEFAULT_BROWSER_FORM: BrowserFormData = {
  enabled: true,
  launch_mode: 'auto_launch_headless',
  chrome_path: '',
  local_cdp_port: 9222,
  use_user_profile: false,
  cdp_url: 'http://127.0.0.1:9222',
  timeout_ms: 30000,
  allowed_domains: ['*'],
  block_private_networks: true,
  max_screenshot_dim: 4096,
};

export const DEFAULT_STORAGE_FORM: StorageFormData = {
  db_path: 'data/y-agent.db',
  pool_size: 5,
  wal_enabled: true,
  busy_timeout_ms: 5000,
  transcript_dir: 'data/transcripts',
};

export const DEFAULT_HOOKS_FORM: HooksFormData = {
  middleware_timeout_ms: 30000,
  event_channel_capacity: 1024,
  max_subscribers: 64,
};

export const DEFAULT_TOOLS_FORM: ToolsFormData = {
  max_active: 20,
  search_limit: 10,
  allow_dynamic_tools: false,
};

export const DEFAULT_GUARDRAILS_FORM: GuardrailsFormData = {
  default_permission: 'notify',
  dangerous_auto_ask: true,
  max_tool_iterations: 50,
  loop_guard_max_iterations: 50,
  loop_guard_similarity_threshold: 0.95,
  risk_high_risk_threshold: 0.8,
  hitl_auto_approve_low_risk: true,
};

export const DEFAULT_KNOWLEDGE_FORM: KnowledgeFormData = {
  l0_max_tokens: 200,
  l1_max_tokens: 500,
  l2_max_tokens: 500,
  max_chunks_per_entry: 5000,
  default_collection: 'default',
  min_similarity_threshold: 0.65,
  embedding_enabled: true,
  embedding_model: '',
  embedding_dimensions: 1536,
  embedding_base_url: '',
  embedding_api_key_env: '',
  embedding_api_key: '',
  embedding_max_tokens: 0,
  retrieval_strategy: 'hybrid',
  bm25_weight: 1.0,
  vector_weight: 1.0,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Mask sensitive values in TOML content. */
export function maskSensitive(content: string): string {
  return content.replace(
    /^(\s*(?:api_key|password|secret|token)\s*=\s*)"([^"]+)"/gm,
    (_match, prefix, value) => `${prefix}"${'*'.repeat(Math.min(value.length, 24))}"`,
  ).replace(
    /^(\s*"(?:authorization|api-key|x-api-key|x-api-token)"\s*=\s*)"([^"]+)"/gim,
    (_match, prefix, value) => `${prefix}"${'*'.repeat(Math.min(value.length, 24))}"`,
  );
}

/** Convert ProviderFormData[] to TOML string. */
export function providersToToml(providers: ProviderFormData[]): string {
  const lines: string[] = [];
  for (const p of providers) {
    lines.push('[[providers]]');
    lines.push(`id = "${escapeTomlString(p.id)}"`);
    lines.push(`provider_type = "${escapeTomlString(p.provider_type)}"`);
    lines.push(`model = "${escapeTomlString(p.model)}"`);
    if (!p.enabled) lines.push('enabled = false');
    if (p.tags.length > 0) {
      lines.push(`tags = [${p.tags.map((t) => `"${escapeTomlString(t)}"`).join(', ')}]`);
    }
    if (p.capabilities.length > 0) {
      lines.push(
        `capabilities = [${p.capabilities.map((c) => `"${escapeTomlString(c)}"`).join(', ')}]`,
      );
    }
    lines.push(`max_concurrency = ${p.max_concurrency}`);
    lines.push(`context_window = ${p.context_window}`);
    if (p.cost_per_1k_input > 0) lines.push(`cost_per_1k_input = ${p.cost_per_1k_input}`);
    if (p.cost_per_1k_output > 0) lines.push(`cost_per_1k_output = ${p.cost_per_1k_output}`);
    if (p.api_key) lines.push(`api_key = "${escapeTomlString(p.api_key)}"`);
    if (p.api_key_env) lines.push(`api_key_env = "${escapeTomlString(p.api_key_env)}"`);
    if (p.base_url) lines.push(`base_url = "${escapeTomlString(p.base_url)}"`);
    if (p.http_protocol === 'http2') lines.push('http_protocol = "http2"');
    if (p.temperature !== null) lines.push(`temperature = ${p.temperature}`);
    if (p.top_p !== null) lines.push(`top_p = ${p.top_p}`);
    if (p.tool_calling_mode) lines.push(`tool_calling_mode = "${escapeTomlString(p.tool_calling_mode)}"`);
    if (p.icon) lines.push(`icon = "${escapeTomlString(p.icon)}"`);
    const headerEntries = Object.entries(p.headers ?? {}).filter(([key]) => key.trim() !== '');
    if (headerEntries.length > 0) {
      lines.push('[providers.headers]');
      for (const [key, value] of headerEntries) {
        lines.push(`"${escapeTomlString(key)}" = "${escapeTomlString(value)}"`);
      }
    }
    lines.push('');
  }
  return lines.join('\n');
}

function stringRecord(value: unknown): Record<string, string> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, val]) => [key, String(val ?? '')]),
  );
}

export function jsonToProviders(json: unknown): ProviderFormData[] {
  // config_get nests each section's parsed TOML under the section name.
  // providers.toml parses to { providers: [...], ...meta }, then gets stored as
  // merged["providers"], so the actual array lives at json.providers.providers.
  // Fall back to json.providers directly for forward-compatibility.
  const jsonObj = json as Record<string, unknown> | null;
  let list: unknown = null;
  if (jsonObj && Array.isArray(jsonObj.providers)) {
    list = jsonObj.providers;
  } else if (jsonObj && jsonObj.providers && typeof jsonObj.providers === 'object' && Array.isArray((jsonObj.providers as Record<string, unknown>).providers)) {
    list = (jsonObj.providers as Record<string, unknown>).providers;
  }
  if (!list || !Array.isArray(list)) return [];
  return list.map((p: Record<string, unknown>) => ({
    id: (p.id as string) ?? '',
    provider_type: (p.provider_type as string) ?? 'openai',
    model: (p.model as string) ?? '',
    enabled: p.enabled !== false,
    tags: Array.isArray(p.tags) ? (p.tags as string[]) : [],
    capabilities: Array.isArray(p.capabilities) ? (p.capabilities as string[]) : [],
    max_concurrency: (p.max_concurrency as number) ?? 5,
    context_window: (p.context_window as number) ?? 128000,
    cost_per_1k_input: (p.cost_per_1k_input as number) ?? 0,
    cost_per_1k_output: (p.cost_per_1k_output as number) ?? 0,
    api_key: (p.api_key as string) ?? null,
    api_key_env: (p.api_key_env as string) ?? null,
    base_url: (p.base_url as string) ?? null,
    headers: stringRecord(p.headers),
    http_protocol: p.http_protocol === 'http2' ? 'http2' : 'http1',
    temperature: (p.temperature as number) ?? null,
    top_p: (p.top_p as number) ?? null,
    tool_calling_mode: (p.tool_calling_mode as string) ?? null,
    icon: (p.icon as string) ?? null,
  }));
}

export function jsonToSession(json: unknown): SessionFormData {
  return deserializeFromJson(json, SESSION_SCHEMA) as unknown as SessionFormData;
}

export function jsonToRuntime(json: unknown): RuntimeFormData {
  return deserializeFromJson(json, RUNTIME_SCHEMA) as unknown as RuntimeFormData;
}

export function jsonToBrowser(json: unknown): BrowserFormData {
  return deserializeFromJson(json, BROWSER_SCHEMA, browserPostProcess) as unknown as BrowserFormData;
}

export function jsonToMcpServers(json: unknown): McpServerFormData[] {
  const jsonObj = json as Record<string, unknown> | null;
  const servers = (jsonObj?.mcpServers as Record<string, unknown>) ?? {};
  return Object.entries(servers).map(([name, cfgRaw]: [string, unknown]) => {
    const cfg = cfgRaw as Record<string, unknown> | null;
    const explicitTransport = cfg?.transport as string | undefined;
    const transport: 'stdio' | 'http' =
      explicitTransport === 'http' || explicitTransport === 'sse'
        ? 'http'
        : explicitTransport === 'stdio'
          ? 'stdio'
          : cfg?.url
            ? 'http'
            : 'stdio';
    return {
      name,
      transport,
      command: (cfg?.command as string) ?? '',
      args: Array.isArray(cfg?.args) ? (cfg.args as string[]) : [],
      env: (cfg?.env as Record<string, string>) ?? {},
      url: (cfg?.url as string) ?? '',
      headers: (cfg?.headers as Record<string, string>) ?? {},
      bearer_token: (cfg?.bearer_token as string) ?? '',
      cwd: (cfg?.cwd as string) ?? '',
      startup_timeout_secs: typeof cfg?.startup_timeout_secs === 'number' ? (cfg.startup_timeout_secs as number) : 30,
      tool_timeout_secs: typeof cfg?.tool_timeout_secs === 'number' ? (cfg.tool_timeout_secs as number) : 120,
      alwaysAllow: Array.isArray(cfg?.alwaysAllow) ? (cfg.alwaysAllow as string[]) : [],
      disabled: (cfg?.disabled as boolean) ?? false,
    };
  });
}

export function mcpServersToJson(servers: McpServerFormData[]): Record<string, unknown> {
  const mcpServers: Record<string, unknown> = {};
  for (const s of servers) {
    const name = s.name || `server-${Object.keys(mcpServers).length + 1}`;
    const entry: Record<string, unknown> = {
      transport: s.transport,
      disabled: s.disabled,
      startup_timeout_secs: s.startup_timeout_secs,
      tool_timeout_secs: s.tool_timeout_secs,
    };
    if (s.transport === 'stdio') {
      entry.command = s.command;
      entry.args = s.args;
      if (Object.keys(s.env).length > 0) entry.env = s.env;
      if (s.cwd) entry.cwd = s.cwd;
    } else {
      entry.url = s.url;
      if (Object.keys(s.headers).length > 0) entry.headers = s.headers;
      if (s.bearer_token) entry.bearer_token = s.bearer_token;
    }
    if (s.alwaysAllow.length > 0) entry.alwaysAllow = s.alwaysAllow;
    mcpServers[name] = entry;
  }
  return { mcpServers };
}

export function sessionToToml(form: SessionFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, SESSION_SCHEMA);
}

export function runtimeToToml(form: RuntimeFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, RUNTIME_SCHEMA);
}

export function browserToToml(form: BrowserFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, BROWSER_SCHEMA);
}

export function jsonToStorage(json: unknown): StorageFormData {
  return deserializeFromJson(json, STORAGE_SCHEMA) as unknown as StorageFormData;
}

export function storageToToml(form: StorageFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, STORAGE_SCHEMA);
}

export function jsonToHooks(json: unknown): HooksFormData {
  return deserializeFromJson(json, HOOKS_SCHEMA) as unknown as HooksFormData;
}

export function hooksToToml(form: HooksFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, HOOKS_SCHEMA);
}

export function jsonToTools(json: unknown): ToolsFormData {
  return deserializeFromJson(json, TOOLS_SCHEMA) as unknown as ToolsFormData;
}

export function toolsToToml(form: ToolsFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, TOOLS_SCHEMA);
}

export function jsonToGuardrails(json: unknown): GuardrailsFormData {
  return deserializeFromJson(json, GUARDRAILS_SCHEMA) as unknown as GuardrailsFormData;
}

export function guardrailsToToml(form: GuardrailsFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, GUARDRAILS_SCHEMA);
}

export function jsonToKnowledge(json: unknown): KnowledgeFormData {
  return deserializeFromJson(json, KNOWLEDGE_SCHEMA) as unknown as KnowledgeFormData;
}

export function knowledgeToToml(form: KnowledgeFormData, rawToml: string | undefined): string {
  return mergeIntoRawToml(rawToml, form as unknown as Record<string, unknown>, KNOWLEDGE_SCHEMA);
}

export const CONFIG_SECTIONS: { key: SettingsTab; label: string }[] = [
  { key: 'providers', label: 'Providers' },
  { key: 'session', label: 'Session' },
  { key: 'runtime', label: 'Runtime' },
  { key: 'browser', label: 'Browser' },
  { key: 'mcp', label: 'MCP Servers' },
  { key: 'storage', label: 'Storage' },
  { key: 'hooks', label: 'Hooks' },
  { key: 'tools', label: 'Tools' },
  { key: 'guardrails', label: 'Guardrails' },
  { key: 'knowledge', label: 'Knowledge' },
  { key: 'promptTemplates', label: 'Prompt Templates' },
];

export const TAB_LABELS: Record<SettingsTab, string> = {
  general: 'General',
  providers: 'Providers',
  session: 'Session',
  runtime: 'Runtime',
  browser: 'Browser',
  mcp: 'MCP Servers',
  storage: 'Storage',
  hooks: 'Hooks',
  tools: 'Tools',
  guardrails: 'Guardrails',
  knowledge: 'Knowledge',
  promptTemplates: 'Prompt Templates',
  prompts: 'Builtin Prompts',
  about: 'About',
};
