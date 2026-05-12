// ---------------------------------------------------------------------------
// Declarative field schemas for each TOML config section.
//
// Used with serializeToml / deserializeFromJson from tomlUtils.ts.
// ---------------------------------------------------------------------------

import type { FieldDef } from './tomlUtils';

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

// ---------------------------------------------------------------------------
// Session schema  (session.toml)
// ---------------------------------------------------------------------------

export const SESSION_SCHEMA: FieldDef[] = [
  // Root-level fields
  { formKey: 'max_depth', tomlKey: 'max_depth', type: 'number', defaultValue: 16 },
  { formKey: 'max_active_per_root', tomlKey: 'max_active_per_root', type: 'number', defaultValue: 8 },
  { formKey: 'compaction_threshold_pct', tomlKey: 'compaction_threshold_pct', type: 'number', defaultValue: 85 },
  { formKey: 'auto_archive_merged', tomlKey: 'auto_archive_merged', type: 'boolean', defaultValue: true },

  // [pruning]
  { formKey: 'pruning_enabled', tomlKey: 'enabled', section: 'pruning', type: 'boolean', defaultValue: true },
  { formKey: 'pruning_token_threshold', tomlKey: 'token_threshold', section: 'pruning', type: 'number', defaultValue: 2000 },
  { formKey: 'pruning_strategy', tomlKey: 'strategy', section: 'pruning', type: 'string', defaultValue: 'auto' },

  // [pruning.progressive]
  { formKey: 'pruning_progressive_max_retries', tomlKey: 'max_retries', section: 'pruning.progressive', type: 'number', defaultValue: 2 },
  { formKey: 'pruning_progressive_preserve_identifiers', tomlKey: 'preserve_identifiers', section: 'pruning.progressive', type: 'boolean', defaultValue: true },
];

// ---------------------------------------------------------------------------
// Browser schema  (browser.toml)
// ---------------------------------------------------------------------------

export const BROWSER_SCHEMA: FieldDef[] = [
  { formKey: 'enabled', tomlKey: 'enabled', type: 'boolean', defaultValue: true },
  { formKey: 'launch_mode', tomlKey: 'launch_mode', type: 'string', defaultValue: 'auto_launch_headless' },
  { formKey: 'chrome_path', tomlKey: 'chrome_path', type: 'string', defaultValue: '', optional: true },
  { formKey: 'local_cdp_port', tomlKey: 'local_cdp_port', type: 'number', defaultValue: 9222 },
  { formKey: 'use_user_profile', tomlKey: 'use_user_profile', type: 'boolean', defaultValue: false },
  { formKey: 'cdp_url', tomlKey: 'cdp_url', type: 'string', defaultValue: 'http://127.0.0.1:9222' },
  { formKey: 'timeout_ms', tomlKey: 'timeout_ms', type: 'number', defaultValue: 30000 },
  { formKey: 'allowed_domains', tomlKey: 'allowed_domains', type: 'string[]', defaultValue: ['*'] },
  { formKey: 'block_private_networks', tomlKey: 'block_private_networks', type: 'boolean', defaultValue: true },
  { formKey: 'max_screenshot_dim', tomlKey: 'max_screenshot_dim', type: 'number', defaultValue: 4096 },
];

// ---------------------------------------------------------------------------
// Runtime schema  (runtime.toml)
// ---------------------------------------------------------------------------

const VOLUME_SUB_SCHEMA: FieldDef[] = [
  { formKey: 'host_path', tomlKey: 'host_path', type: 'string', defaultValue: '' },
  { formKey: 'container_path', tomlKey: 'container_path', type: 'string', defaultValue: '' },
  { formKey: 'mode', tomlKey: 'mode', type: 'string', defaultValue: 'ro' },
];

export const RUNTIME_SCHEMA: FieldDef[] = [
  // Root-level
  { formKey: 'default_backend', tomlKey: 'default_backend', type: 'string', defaultValue: 'native' },
  { formKey: 'allow_shell', tomlKey: 'allow_shell', type: 'boolean', defaultValue: false },
  { formKey: 'allow_host_access', tomlKey: 'allow_host_access', type: 'boolean', defaultValue: false },
  { formKey: 'default_timeout', tomlKey: 'default_timeout', type: 'string', defaultValue: '30s' },
  { formKey: 'default_memory_bytes', tomlKey: 'default_memory_bytes', type: 'number', defaultValue: 536870912 },

  // [ssh]
  { formKey: 'ssh_host', tomlKey: 'host', section: 'ssh', type: 'string', defaultValue: 'localhost' },
  { formKey: 'ssh_port', tomlKey: 'port', section: 'ssh', type: 'number', defaultValue: 22 },
  { formKey: 'ssh_user', tomlKey: 'user', section: 'ssh', type: 'string', defaultValue: 'root' },
  { formKey: 'ssh_auth_method', tomlKey: 'auth_method', section: 'ssh', type: 'string', defaultValue: 'public_key' },
  { formKey: 'ssh_password', tomlKey: 'password', section: 'ssh', type: 'string', defaultValue: '', optional: true },
  { formKey: 'ssh_private_key_path', tomlKey: 'private_key_path', section: 'ssh', type: 'string', defaultValue: '', optional: true },
  { formKey: 'ssh_passphrase', tomlKey: 'passphrase', section: 'ssh', type: 'string', defaultValue: '', optional: true },
  { formKey: 'ssh_known_hosts_path', tomlKey: 'known_hosts_path', section: 'ssh', type: 'string', defaultValue: '', optional: true },

  // [docker]
  { formKey: 'docker_default_image', tomlKey: 'default_image', section: 'docker', type: 'string', defaultValue: '', optional: true },
  { formKey: 'docker_network_mode', tomlKey: 'network_mode', section: 'docker', type: 'string', defaultValue: 'none' },
  { formKey: 'docker_privileged', tomlKey: 'privileged', section: 'docker', type: 'boolean', defaultValue: false },
  { formKey: 'docker_readonly_rootfs', tomlKey: 'readonly_rootfs', section: 'docker', type: 'boolean', defaultValue: true },
  { formKey: 'docker_user', tomlKey: 'user', section: 'docker', type: 'string', defaultValue: '', optional: true },
  { formKey: 'docker_cap_drop', tomlKey: 'cap_drop', section: 'docker', type: 'string[]', defaultValue: ['ALL'], optional: true },
  { formKey: 'docker_cap_add', tomlKey: 'cap_add', section: 'docker', type: 'string[]', defaultValue: [], optional: true },
  { formKey: 'docker_dns', tomlKey: 'dns', section: 'docker', type: 'string[]', defaultValue: [], optional: true },
  { formKey: 'docker_extra_hosts', tomlKey: 'extra_hosts', section: 'docker', type: 'string[]', defaultValue: [], optional: true },

  // [docker.default_env]  (record)
  { formKey: 'docker_default_env', tomlKey: 'default_env', section: 'docker', type: 'record', defaultValue: {}, optional: true },

  // [[docker.default_volumes]]  (array of tables)
  {
    formKey: 'docker_default_volumes',
    tomlKey: 'default_volumes',
    section: 'docker',
    type: 'table[]',
    defaultValue: [],
    optional: true,
    subSchema: VOLUME_SUB_SCHEMA,
  },

  // [python_venv]
  { formKey: 'python_venv_enabled', tomlKey: 'enabled', section: 'python_venv', type: 'boolean', defaultValue: false },
  { formKey: 'python_uv_path', tomlKey: 'uv_path', section: 'python_venv', type: 'string', defaultValue: 'uv' },
  { formKey: 'python_version', tomlKey: 'python_version', section: 'python_venv', type: 'string', defaultValue: '3.12' },
  { formKey: 'python_venv_dir', tomlKey: 'venv_dir', section: 'python_venv', type: 'string', defaultValue: '.venv' },
  { formKey: 'python_working_dir', tomlKey: 'working_dir', section: 'python_venv', type: 'string', defaultValue: '', optional: true },

  // [bun_venv]
  { formKey: 'bun_venv_enabled', tomlKey: 'enabled', section: 'bun_venv', type: 'boolean', defaultValue: false },
  { formKey: 'bun_path', tomlKey: 'bun_path', section: 'bun_venv', type: 'string', defaultValue: 'bun' },
  { formKey: 'bun_version', tomlKey: 'bun_version', section: 'bun_venv', type: 'string', defaultValue: 'latest' },
  { formKey: 'bun_working_dir', tomlKey: 'working_dir', section: 'bun_venv', type: 'string', defaultValue: '', optional: true },
];

/** Post-processor for browser JSON deserialization.
 *  Translates legacy `auto_launch` + `headless` booleans to `launch_mode`. */
export function browserPostProcess(
  result: Record<string, unknown>,
  json: unknown,
): void {
  if (!isJsonObject(json)) return;

  if (!json.launch_mode && json.auto_launch) {
    result.launch_mode = json.headless === false
      ? 'auto_launch_visible'
      : 'auto_launch_headless';
  }
}

/** Post-processor for runtime JSON deserialization.
 *  Handles conditional SSH fields based on auth_method. */
export function runtimePostProcess(result: Record<string, unknown>): void {
  // Nothing extra needed: the schema already handles all fields.
  // Kept as a placeholder for future auth-method-dependent logic.
  void result;
}

// ---------------------------------------------------------------------------
// Storage schema  (storage.toml)
// ---------------------------------------------------------------------------

export const STORAGE_SCHEMA: FieldDef[] = [
  { formKey: 'db_path', tomlKey: 'db_path', type: 'string', defaultValue: 'data/y-agent.db' },
  { formKey: 'pool_size', tomlKey: 'pool_size', type: 'number', defaultValue: 5 },
  { formKey: 'wal_enabled', tomlKey: 'wal_enabled', type: 'boolean', defaultValue: true },
  { formKey: 'busy_timeout_ms', tomlKey: 'busy_timeout_ms', type: 'number', defaultValue: 5000 },
  { formKey: 'transcript_dir', tomlKey: 'transcript_dir', type: 'string', defaultValue: 'data/transcripts' },
];

// ---------------------------------------------------------------------------
// Hooks schema  (hooks.toml)
// ---------------------------------------------------------------------------

export const HOOKS_SCHEMA: FieldDef[] = [
  { formKey: 'middleware_timeout_ms', tomlKey: 'middleware_timeout_ms', type: 'number', defaultValue: 30000 },
  { formKey: 'event_channel_capacity', tomlKey: 'event_channel_capacity', type: 'number', defaultValue: 1024 },
  { formKey: 'max_subscribers', tomlKey: 'max_subscribers', type: 'number', defaultValue: 64 },
];

// ---------------------------------------------------------------------------
// Tools schema  (tools.toml)
// ---------------------------------------------------------------------------

export const TOOLS_SCHEMA: FieldDef[] = [
  { formKey: 'max_active', tomlKey: 'max_active', type: 'number', defaultValue: 20 },
  { formKey: 'search_limit', tomlKey: 'search_limit', type: 'number', defaultValue: 10 },
  { formKey: 'allow_dynamic_tools', tomlKey: 'allow_dynamic_tools', type: 'boolean', defaultValue: false },
];

// ---------------------------------------------------------------------------
// Guardrails schema  (guardrails.toml)
// ---------------------------------------------------------------------------

export const GUARDRAILS_SCHEMA: FieldDef[] = [
  // Root-level
  { formKey: 'default_permission', tomlKey: 'default_permission', type: 'string', defaultValue: 'notify' },
  { formKey: 'dangerous_auto_ask', tomlKey: 'dangerous_auto_ask', type: 'boolean', defaultValue: true },
  { formKey: 'max_tool_iterations', tomlKey: 'max_tool_iterations', type: 'number', defaultValue: 50 },

  // [plan_review]
  { formKey: 'plan_review_mode', tomlKey: 'mode', section: 'plan_review', type: 'string', defaultValue: 'manual', optional: true },

  // [loop_guard]
  { formKey: 'loop_guard_max_iterations', tomlKey: 'max_iterations', section: 'loop_guard', type: 'number', defaultValue: 50, optional: true },
  { formKey: 'loop_guard_similarity_threshold', tomlKey: 'similarity_threshold', section: 'loop_guard', type: 'number', defaultValue: 0.95, optional: true },

  // [risk]
  { formKey: 'risk_high_risk_threshold', tomlKey: 'high_risk_threshold', section: 'risk', type: 'number', defaultValue: 0.8, optional: true },

  // [hitl]
  { formKey: 'hitl_auto_approve_low_risk', tomlKey: 'auto_approve_low_risk', section: 'hitl', type: 'boolean', defaultValue: true, optional: true },
];

// ---------------------------------------------------------------------------
// Knowledge schema  (knowledge.toml)
// ---------------------------------------------------------------------------

export const KNOWLEDGE_SCHEMA: FieldDef[] = [
  // Chunking
  { formKey: 'l0_max_tokens', tomlKey: 'l0_max_tokens', type: 'number', defaultValue: 200 },
  { formKey: 'l1_max_tokens', tomlKey: 'l1_max_tokens', type: 'number', defaultValue: 500 },
  { formKey: 'l2_max_tokens', tomlKey: 'l2_max_tokens', type: 'number', defaultValue: 500 },
  { formKey: 'max_chunks_per_entry', tomlKey: 'max_chunks_per_entry', type: 'number', defaultValue: 5000 },
  { formKey: 'default_collection', tomlKey: 'default_collection', type: 'string', defaultValue: 'default' },
  { formKey: 'min_similarity_threshold', tomlKey: 'min_similarity_threshold', type: 'number', defaultValue: 0.65 },

  // Embedding
  { formKey: 'embedding_enabled', tomlKey: 'embedding_enabled', type: 'boolean', defaultValue: true },
  { formKey: 'embedding_model', tomlKey: 'embedding_model', type: 'string', defaultValue: '' },
  { formKey: 'embedding_dimensions', tomlKey: 'embedding_dimensions', type: 'number', defaultValue: 1536 },
  { formKey: 'embedding_base_url', tomlKey: 'embedding_base_url', type: 'string', defaultValue: '', optional: true },
  { formKey: 'embedding_api_key_env', tomlKey: 'embedding_api_key_env', type: 'string', defaultValue: '', optional: true },
  { formKey: 'embedding_api_key', tomlKey: 'embedding_api_key', type: 'string', defaultValue: '', optional: true },
  { formKey: 'embedding_max_tokens', tomlKey: 'embedding_max_tokens', type: 'number', defaultValue: 0 },

  // Retrieval
  { formKey: 'retrieval_strategy', tomlKey: 'retrieval_strategy', type: 'string', defaultValue: 'hybrid' },
  { formKey: 'bm25_weight', tomlKey: 'bm25_weight', type: 'number', defaultValue: 1.0 },
  { formKey: 'vector_weight', tomlKey: 'vector_weight', type: 'number', defaultValue: 1.0 },
];

// ---------------------------------------------------------------------------
// Langfuse schema  (langfuse.toml)
// ---------------------------------------------------------------------------

export const LANGFUSE_SCHEMA: FieldDef[] = [
  // Root-level
  { formKey: 'enabled', tomlKey: 'enabled', type: 'boolean', defaultValue: false },
  { formKey: 'base_url', tomlKey: 'base_url', type: 'string', defaultValue: 'https://cloud.langfuse.com' },
  { formKey: 'public_key', tomlKey: 'public_key', type: 'string', defaultValue: '' },
  { formKey: 'secret_key', tomlKey: 'secret_key', type: 'string', defaultValue: '' },
  { formKey: 'project_id', tomlKey: 'project_id', type: 'string', defaultValue: '', optional: true },

  // [content]
  { formKey: 'content_capture_input', tomlKey: 'capture_input', section: 'content', type: 'boolean', defaultValue: false },
  { formKey: 'content_capture_output', tomlKey: 'capture_output', section: 'content', type: 'boolean', defaultValue: false },
  { formKey: 'content_max_content_length', tomlKey: 'max_content_length', section: 'content', type: 'number', defaultValue: 10000 },

  // [redaction]
  { formKey: 'redaction_enabled', tomlKey: 'enabled', section: 'redaction', type: 'boolean', defaultValue: true },
  { formKey: 'redaction_patterns', tomlKey: 'patterns', section: 'redaction', type: 'string[]', defaultValue: [
    '(?i)(api[_-]?key|secret|token|password|bearer)\\s*[:=]\\s*\\S+',
    '\\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Z|a-z]{2,}\\b',
  ] },
  { formKey: 'redaction_replacement', tomlKey: 'replacement', section: 'redaction', type: 'string', defaultValue: '[REDACTED]' },

  // [sampling]
  { formKey: 'sampling_rate', tomlKey: 'rate', section: 'sampling', type: 'number', defaultValue: 1.0 },
  { formKey: 'sampling_include_tags', tomlKey: 'include_tags', section: 'sampling', type: 'string[]', defaultValue: [] },
  { formKey: 'sampling_exclude_tags', tomlKey: 'exclude_tags', section: 'sampling', type: 'string[]', defaultValue: [] },

  // [retry]
  { formKey: 'retry_max_retries', tomlKey: 'max_retries', section: 'retry', type: 'number', defaultValue: 3, optional: true },
  { formKey: 'retry_initial_backoff_ms', tomlKey: 'initial_backoff_ms', section: 'retry', type: 'number', defaultValue: 1000, optional: true },
  { formKey: 'retry_max_backoff_ms', tomlKey: 'max_backoff_ms', section: 'retry', type: 'number', defaultValue: 30000, optional: true },

  // [feedback]
  { formKey: 'feedback_import_enabled', tomlKey: 'import_enabled', section: 'feedback', type: 'boolean', defaultValue: false, optional: true },
  { formKey: 'feedback_poll_interval_secs', tomlKey: 'poll_interval_secs', section: 'feedback', type: 'number', defaultValue: 300, optional: true },

  // [circuit_breaker]
  { formKey: 'circuit_breaker_failure_threshold', tomlKey: 'failure_threshold', section: 'circuit_breaker', type: 'number', defaultValue: 5, optional: true },
  { formKey: 'circuit_breaker_recovery_timeout_secs', tomlKey: 'recovery_timeout_secs', section: 'circuit_breaker', type: 'number', defaultValue: 60, optional: true },
];
