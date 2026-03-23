// ---------------------------------------------------------------------------
// Declarative field schemas for each TOML config section.
//
// Used with serializeToml / deserializeFromJson from tomlUtils.ts.
// ---------------------------------------------------------------------------

import type { FieldDef } from './tomlUtils';

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
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function browserPostProcess(result: Record<string, any>, json: any): void {
  if (!json?.launch_mode && json?.auto_launch) {
    result.launch_mode = json?.headless === false
      ? 'auto_launch_visible'
      : 'auto_launch_headless';
  }
}

/** Post-processor for runtime JSON deserialization.
 *  Handles conditional SSH fields based on auth_method. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function runtimePostProcess(result: Record<string, any>): void {
  // Nothing extra needed: the schema already handles all fields.
  // Kept as a placeholder for future auth-method-dependent logic.
  void result;
}
