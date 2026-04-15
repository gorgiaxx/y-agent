// ---------------------------------------------------------------------------
// RuntimeTab -- Runtime configuration form (native, SSH, Docker, Python, Bun)
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TagChipInput } from './TagChipInput';
import type { RuntimeFormData } from './settingsTypes';
import { jsonToRuntime } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { RUNTIME_SCHEMA } from '../../utils/settingsSchemas';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input } from '../ui';

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
  const cachedRawToml = useRef<string | undefined>(undefined);

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
        cachedRawToml.current = raw;
      } catch {
        setRawRuntimeToml(undefined);
        cachedRawToml.current = undefined;
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
      setRawContent(mergeIntoRawToml(cachedRawToml.current, runtimeForm as unknown as Record<string, unknown>, RUNTIME_SCHEMA));
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
      {/* General */}
      <div className="pf-row pf-row-triple">
        <div className="pf-field">
          <label className="pf-label">Default Backend</label>
          <Select
            value={runtimeForm.default_backend}
            onValueChange={(val) => { setRuntimeForm({ ...runtimeForm, default_backend: val }); setDirtyRuntime(true); }}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select backend" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="native">Native</SelectItem>
              <SelectItem value="docker">Docker</SelectItem>
              <SelectItem value="ssh">SSH</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="pf-field">
          <label className="pf-label">Default Timeout</label>
          <Input
            value={runtimeForm.default_timeout}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, default_timeout: e.target.value }); setDirtyRuntime(true); }}
            placeholder="e.g. 30s, 5m"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Memory Limit (bytes)</label>
          <Input
            numeric
            type="number"
            min={0}
            step={1048576}
            value={runtimeForm.default_memory_bytes}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, default_memory_bytes: Number(e.target.value) || 536870912 }); setDirtyRuntime(true); }}
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.allow_shell}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, allow_shell: c === true }); setDirtyRuntime(true); }}
            />
            {' '}Allow shell execution
          </label>
        </div>
        <div className="pf-field">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.allow_host_access}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, allow_host_access: c === true }); setDirtyRuntime(true); }}
            />
            {' '}Allow host filesystem access
          </label>
        </div>
      </div>

      {/* --- SSH section --- */}
      <div className="pf-section-divider">
        <span className="pf-section-title">SSH Configuration</span>
      </div>
      <div className="pf-row pf-row-triple">
        <div className="pf-field">
          <label className="pf-label">Host</label>
          <Input
            value={runtimeForm.ssh_host}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_host: e.target.value }); setDirtyRuntime(true); }}
            placeholder="localhost"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Port</label>
          <Input
            numeric
            type="number"
            min={1}
            max={65535}
            value={runtimeForm.ssh_port}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_port: Number(e.target.value) || 22 }); setDirtyRuntime(true); }}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">User</label>
          <Input
            value={runtimeForm.ssh_user}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_user: e.target.value }); setDirtyRuntime(true); }}
            placeholder="root"
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Auth Method</label>
          <Select
            value={runtimeForm.ssh_auth_method}
            onValueChange={(val) => { setRuntimeForm({ ...runtimeForm, ssh_auth_method: val }); setDirtyRuntime(true); }}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select auth method" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="public_key">Public Key</SelectItem>
              <SelectItem value="password">Password</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {runtimeForm.ssh_auth_method === 'password' ? (
          <div className="pf-field">
            <label className="pf-label">Password</label>
            <Input
              type="password"
              value={runtimeForm.ssh_password}
              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_password: e.target.value }); setDirtyRuntime(true); }}
              placeholder="SSH password"
            />
          </div>
        ) : (
          <div className="pf-field">
            <label className="pf-label">Private Key Path</label>
            <Input
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
            <Input
              type="password"
              value={runtimeForm.ssh_passphrase}
              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_passphrase: e.target.value }); setDirtyRuntime(true); }}
              placeholder="(optional)"
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Known Hosts Path</label>
            <Input
              value={runtimeForm.ssh_known_hosts_path}
              onChange={(e) => { setRuntimeForm({ ...runtimeForm, ssh_known_hosts_path: e.target.value }); setDirtyRuntime(true); }}
              placeholder="~/.ssh/known_hosts"
            />
          </div>
        </div>
      )}

      {/* --- Docker section --- */}
      <div className="pf-section-divider">
        <span className="pf-section-title">Docker Configuration</span>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Default Image</label>
          <Input
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
          <Select
            value={runtimeForm.docker_network_mode}
            onValueChange={(val) => { setRuntimeForm({ ...runtimeForm, docker_network_mode: val }); setDirtyRuntime(true); }}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select network mode" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="none">none</SelectItem>
              <SelectItem value="bridge">bridge</SelectItem>
              <SelectItem value="host">host</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="pf-field">
          <label className="pf-label">Container User</label>
          <Input
            value={runtimeForm.docker_user}
            onChange={(e) => { setRuntimeForm({ ...runtimeForm, docker_user: e.target.value }); setDirtyRuntime(true); }}
            placeholder="e.g. 1000:1000"
          />
        </div>
      </div>
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.docker_privileged}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, docker_privileged: c === true }); setDirtyRuntime(true); }}
            />
            {' '}Privileged mode
          </label>
        </div>
        <div className="pf-field">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.docker_readonly_rootfs}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, docker_readonly_rootfs: c === true }); setDirtyRuntime(true); }}
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
          <div className="pf-kv-list">
            {Object.entries(runtimeForm.docker_default_env).map(([k, v], i) => (
              <div key={i} className="pf-kv-row">
                <Input
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
                <span className="pf-kv-sep">=</span>
                <Input
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
              className="btn-test pf-kv-add"
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
          <div className="pf-kv-list">
            {runtimeForm.docker_default_volumes.map((vol, i) => (
              <div key={i} className="pf-kv-row">
                <Input
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
                <span className="pf-kv-sep">-&gt;</span>
                <Input
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
                <Select
                  value={vol.mode}
                  onValueChange={(val) => {
                    const vols = [...runtimeForm.docker_default_volumes];
                    vols[i] = { ...vols[i], mode: val };
                    setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                    setDirtyRuntime(true);
                  }}
                >
                  <SelectTrigger className="w-[70px] min-w-[70px]">
                    <SelectValue placeholder="Mode" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="ro">ro</SelectItem>
                    <SelectItem value="rw">rw</SelectItem>
                  </SelectContent>
                </Select>
                <button
                  type="button"
                  className="pf-tag-chip-remove"
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
              className="btn-test pf-kv-add"
              onClick={() => {
                const vols = [...runtimeForm.docker_default_volumes, { host_path: '', container_path: '', mode: 'ro' }];
                setRuntimeForm({ ...runtimeForm, docker_default_volumes: vols });
                setDirtyRuntime(true);
              }}
            >+ Add Volume</button>
          </div>
        </div>
      </div>

      {/* --- Python Environment (uv) section --- */}
      <div className="pf-section-divider">
        <span className="pf-section-title">Python Environment (uv)</span>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.python_venv_enabled}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, python_venv_enabled: c === true }); setDirtyRuntime(true); }}
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
              <Input
                value={runtimeForm.python_uv_path}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_uv_path: e.target.value }); setDirtyRuntime(true); }}
                placeholder="uv"
              />
            </div>
            <div className="pf-field">
              <label className="pf-label">Python Version</label>
              <Input
                value={runtimeForm.python_version}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_version: e.target.value }); setDirtyRuntime(true); }}
                placeholder="3.12"
              />
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field">
              <label className="pf-label">Venv Directory</label>
              <Input
                value={runtimeForm.python_venv_dir}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_venv_dir: e.target.value }); setDirtyRuntime(true); }}
                placeholder=".venv"
              />
            </div>
            <div className="pf-field">
              <label className="pf-label">Working Directory</label>
              <Input
                value={runtimeForm.python_working_dir}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, python_working_dir: e.target.value }); setDirtyRuntime(true); }}
                placeholder="(uses current dir)"
              />
            </div>
          </div>
        </>
      )}

      {/* --- JavaScript Environment (bun) section --- */}
      <div className="pf-section-divider">
        <span className="pf-section-title">JavaScript Environment (bun)</span>
      </div>
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <Checkbox
              checked={runtimeForm.bun_venv_enabled}
              onCheckedChange={(c) => { setRuntimeForm({ ...runtimeForm, bun_venv_enabled: c === true }); setDirtyRuntime(true); }}
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
              <Input
                value={runtimeForm.bun_path}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_path: e.target.value }); setDirtyRuntime(true); }}
                placeholder="bun"
              />
            </div>
            <div className="pf-field">
              <label className="pf-label">Bun Version</label>
              <Input
                value={runtimeForm.bun_version}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_version: e.target.value }); setDirtyRuntime(true); }}
                placeholder="latest"
              />
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Working Directory</label>
              <Input
                value={runtimeForm.bun_working_dir}
                onChange={(e) => { setRuntimeForm({ ...runtimeForm, bun_working_dir: e.target.value }); setDirtyRuntime(true); }}
                placeholder="(uses current dir)"
              />
            </div>
          </div>
        </>
      )}
    </div>
    </>
  );
}
