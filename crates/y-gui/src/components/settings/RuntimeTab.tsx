// ---------------------------------------------------------------------------
// RuntimeTab -- Runtime configuration form (native, SSH, Docker, Python, Bun)
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TagChipInput } from './TagChipInput';
import type { RuntimeFormData } from './settingsTypes';
import { jsonToRuntime } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { serializeToml } from '../../utils/tomlUtils';
import { RUNTIME_SCHEMA } from '../../utils/settingsSchemas';

interface RuntimeTabProps {
  loadSection: (section: string) => Promise<string>;
  runtimeForm: RuntimeFormData;
  setRuntimeForm: React.Dispatch<React.SetStateAction<RuntimeFormData>>;
  setDirtyRuntime: React.Dispatch<React.SetStateAction<boolean>>;
  setRawRuntimeToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function RuntimeTab({
  loadSection,
  runtimeForm,
  setRuntimeForm,
  setDirtyRuntime,
  setRawRuntimeToml,
}: RuntimeTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');

  const loadRuntimeForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const runtimeJson = allConfig?.runtime ?? {};
      setRuntimeForm(jsonToRuntime(runtimeJson));
      // Cache raw TOML for comment preservation.
      try {
        const raw = await loadSection('runtime');
        setRawRuntimeToml(raw);
      } catch {
        setRawRuntimeToml(undefined);
      }
    } catch {
      // Use defaults if section not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setRuntimeForm, setRawRuntimeToml]);

  useEffect(() => {
    loadRuntimeForm();
  }, [loadRuntimeForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(serializeToml(runtimeForm as unknown as Record<string, unknown>, RUNTIME_SCHEMA));
    }
    setRawMode(next);
  }, [runtimeForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
            <span className="settings-header-with-toggle">Runtime <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawRuntimeToml(val);
            setDirtyRuntime(true);
          }}
          placeholder="No runtime.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
    <div className="settings-header">
      <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
        <span className="settings-header-with-toggle">Runtime <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <div className="settings-form-wrap">
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

      {/* --- SSH section --- */}
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
                  >x</button>
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
                  <span style={{ color: 'var(--text-secondary)' }}>-&gt;</span>
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
                  >x</button>
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
    </>
  );
}
