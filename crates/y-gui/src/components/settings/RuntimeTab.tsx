// ---------------------------------------------------------------------------
// RuntimeTab -- Runtime configuration form (native, SSH, Docker, Python, Bun)
// ---------------------------------------------------------------------------

import { X } from 'lucide-react';
import { TagChipInput } from './TagChipInput';
import type { RuntimeFormData } from './settingsTypes';
import { jsonToRuntime } from './settingsTypes';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
import { RUNTIME_SCHEMA } from '../../utils/settingsSchemas';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, Button, SettingsGroup, SettingsItem } from '../ui';

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
  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'runtime',
    schema: RUNTIME_SCHEMA,
    configKey: 'runtime',
    form: runtimeForm,
    setForm: setRuntimeForm,
    setDirty: setDirtyRuntime,
    setRawToml: setRawRuntimeToml,
    jsonToForm: jsonToRuntime,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No runtime.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="General">
            <SettingsItem title="Default Backend">
              <Select
                value={runtimeForm.default_backend}
                onValueChange={(val) => { update({ default_backend: val }); }}
              >
                <SelectTrigger className="w-[140px]">
                  <SelectValue placeholder="Select backend" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="native">Native</SelectItem>
                  <SelectItem value="docker">Docker</SelectItem>
                  <SelectItem value="ssh">SSH</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
            <SettingsItem title="Default Timeout" wide>
              <Input
                value={runtimeForm.default_timeout}
                onChange={(e) => { update({ default_timeout: e.target.value }); }}
                placeholder="e.g. 30s, 5m"
              />
            </SettingsItem>
            <SettingsItem title="Memory Limit (bytes)">
              <Input
                numeric type="number" min={0} step={1048576} className="w-[140px]"
                value={runtimeForm.default_memory_bytes}
                onChange={(e) => { update({ default_memory_bytes: Number(e.target.value) || 536870912 }); }}
              />
            </SettingsItem>
            <SettingsItem title="Allow shell execution">
              <Checkbox
                checked={runtimeForm.allow_shell}
                onCheckedChange={(c) => { update({ allow_shell: c === true }); }}
              />
            </SettingsItem>
            <SettingsItem title="Allow host filesystem access">
              <Checkbox
                checked={runtimeForm.allow_host_access}
                onCheckedChange={(c) => { update({ allow_host_access: c === true }); }}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="SSH Configuration">
            <SettingsItem title="Host" wide>
              <Input
                value={runtimeForm.ssh_host}
                onChange={(e) => { update({ ssh_host: e.target.value }); }}
                placeholder="localhost"
              />
            </SettingsItem>
            <SettingsItem title="Port">
              <Input
                numeric type="number" min={1} max={65535} className="w-[100px]"
                value={runtimeForm.ssh_port}
                onChange={(e) => { update({ ssh_port: Number(e.target.value) || 22 }); }}
              />
            </SettingsItem>
            <SettingsItem title="User" wide>
              <Input
                value={runtimeForm.ssh_user}
                onChange={(e) => { update({ ssh_user: e.target.value }); }}
                placeholder="root"
              />
            </SettingsItem>
            <SettingsItem title="Auth Method">
              <Select
                value={runtimeForm.ssh_auth_method}
                onValueChange={(val) => { update({ ssh_auth_method: val }); }}
              >
                <SelectTrigger className="w-[140px]">
                  <SelectValue placeholder="Select auth method" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="public_key">Public Key</SelectItem>
                  <SelectItem value="password">Password</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
            {runtimeForm.ssh_auth_method === 'password' ? (
              <SettingsItem title="Password" wide>
                <Input
                  type="password"
                  value={runtimeForm.ssh_password}
                  onChange={(e) => { update({ ssh_password: e.target.value }); }}
                  placeholder="SSH password"
                />
              </SettingsItem>
            ) : (
              <>
                <SettingsItem title="Private Key Path" wide>
                  <Input
                    value={runtimeForm.ssh_private_key_path}
                    onChange={(e) => { update({ ssh_private_key_path: e.target.value }); }}
                    placeholder="~/.ssh/id_rsa"
                  />
                </SettingsItem>
                <SettingsItem title="Passphrase" wide>
                  <Input
                    type="password"
                    value={runtimeForm.ssh_passphrase}
                    onChange={(e) => { update({ ssh_passphrase: e.target.value }); }}
                    placeholder="(optional)"
                  />
                </SettingsItem>
                <SettingsItem title="Known Hosts Path" wide>
                  <Input
                    value={runtimeForm.ssh_known_hosts_path}
                    onChange={(e) => { update({ ssh_known_hosts_path: e.target.value }); }}
                    placeholder="~/.ssh/known_hosts"
                  />
                </SettingsItem>
              </>
            )}
          </SettingsGroup>
          <SettingsGroup title="Docker Configuration">
            <SettingsItem
              title="Default Image"
              description="Container image used for Docker-backend executions when not specified per-request"
              wide
            >
              <Input
                value={runtimeForm.docker_default_image}
                onChange={(e) => { update({ docker_default_image: e.target.value }); }}
                placeholder="e.g. python:3.12-slim, ubuntu:24.04"
              />
            </SettingsItem>
            <SettingsItem title="Network Mode">
              <Select
                value={runtimeForm.docker_network_mode}
                onValueChange={(val) => { update({ docker_network_mode: val }); }}
              >
                <SelectTrigger className="w-[140px]">
                  <SelectValue placeholder="Select network mode" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">none</SelectItem>
                  <SelectItem value="bridge">bridge</SelectItem>
                  <SelectItem value="host">host</SelectItem>
                </SelectContent>
              </Select>
            </SettingsItem>
            <SettingsItem title="Container User" wide>
              <Input
                value={runtimeForm.docker_user}
                onChange={(e) => { update({ docker_user: e.target.value }); }}
                placeholder="e.g. 1000:1000"
              />
            </SettingsItem>
            <SettingsItem title="Privileged mode">
              <Checkbox
                checked={runtimeForm.docker_privileged}
                onCheckedChange={(c) => { update({ docker_privileged: c === true }); }}
              />
            </SettingsItem>
            <SettingsItem title="Read-only root filesystem">
              <Checkbox
                checked={runtimeForm.docker_readonly_rootfs}
                onCheckedChange={(c) => { update({ docker_readonly_rootfs: c === true }); }}
              />
            </SettingsItem>
            <SettingsItem title="Cap Drop" wide>
              <TagChipInput
                tags={runtimeForm.docker_cap_drop}
                onChange={(next) => { update({ docker_cap_drop: next }); }}
              />
            </SettingsItem>
            <SettingsItem title="Cap Add" wide>
              <TagChipInput
                tags={runtimeForm.docker_cap_add}
                onChange={(next) => { update({ docker_cap_add: next }); }}
              />
            </SettingsItem>
            <SettingsItem title="DNS Servers" wide>
              <TagChipInput
                tags={runtimeForm.docker_dns}
                onChange={(next) => { update({ docker_dns: next }); }}
              />
            </SettingsItem>
            <SettingsItem title="Extra Hosts" wide>
              <TagChipInput
                tags={runtimeForm.docker_extra_hosts}
                onChange={(next) => { update({ docker_extra_hosts: next }); }}
              />
            </SettingsItem>
          </SettingsGroup>
          <SettingsGroup title="Docker Environment Variables" bodyVariant="plain">
            <div className="settings-item--custom-body">
              <div className="pf-kv-list">
                {Object.entries(runtimeForm.docker_default_env).map(([k, v], i) => (
                  <div key={i} className="pf-kv-row">
                    <Input
                      style={{ flex: 1 }}
                      value={k}
                      onChange={(e) => {
                        const entries = Object.entries(runtimeForm.docker_default_env);
                        entries[i] = [e.target.value, v];
                        update({ docker_default_env: Object.fromEntries(entries) });
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
                        update({ docker_default_env: newEnv });
                      }}
                      placeholder="value"
                    />
                    <Button
                      variant="icon"
                      size="sm"
                      title="Remove"
                      onClick={() => {
                        const newEnv = { ...runtimeForm.docker_default_env };
                        delete newEnv[k];
                        update({ docker_default_env: newEnv });
                      }}
                    ><X size={12} /></Button>
                  </div>
                ))}
                <Button
                  variant="ghost"
                  size="sm"
                  className="pf-kv-add"
                  onClick={() => {
                    const newEnv = { ...runtimeForm.docker_default_env, '': '' };
                    update({ docker_default_env: newEnv });
                  }}
                >+ Add Variable</Button>
              </div>
            </div>
          </SettingsGroup>

          <SettingsGroup title="Docker Volume Mappings" bodyVariant="plain">
            <div className="settings-item--custom-body">
              <div className="pf-kv-list">
                {runtimeForm.docker_default_volumes.map((vol, i) => (
                  <div key={i} className="pf-kv-row">
                    <Input
                      style={{ flex: 2 }}
                      value={vol.host_path}
                      onChange={(e) => {
                        const vols = [...runtimeForm.docker_default_volumes];
                        vols[i] = { ...vols[i], host_path: e.target.value };
                        update({ docker_default_volumes: vols });
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
                        update({ docker_default_volumes: vols });
                      }}
                      placeholder="Container path"
                    />
                    <Select
                      value={vol.mode}
                      onValueChange={(val) => {
                        const vols = [...runtimeForm.docker_default_volumes];
                        vols[i] = { ...vols[i], mode: val };
                        update({ docker_default_volumes: vols });
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
                    <Button
                      variant="icon"
                      size="sm"
                      title="Remove"
                      onClick={() => {
                        const vols = runtimeForm.docker_default_volumes.filter((_, j) => j !== i);
                        update({ docker_default_volumes: vols });
                      }}
                    ><X size={12} /></Button>
                  </div>
                ))}
                <Button
                  variant="ghost"
                  size="sm"
                  className="pf-kv-add"
                  onClick={() => {
                    const vols = [...runtimeForm.docker_default_volumes, { host_path: '', container_path: '', mode: 'ro' }];
                    update({ docker_default_volumes: vols });
                  }}
                >+ Add Volume</Button>
              </div>
            </div>
          </SettingsGroup>

          <SettingsGroup
            title="Python Environment (uv)"
            description="When enabled, the Python venv path is injected into the system prompt so the LLM uses the correct runtime"
          >
            <SettingsItem title="Enable Python environment">
              <Checkbox
                checked={runtimeForm.python_venv_enabled}
                onCheckedChange={(c) => { update({ python_venv_enabled: c === true }); }}
              />
            </SettingsItem>
            {runtimeForm.python_venv_enabled && (
              <>
                <SettingsItem title="uv Binary Path" wide>
                  <Input
                    value={runtimeForm.python_uv_path}
                    onChange={(e) => { update({ python_uv_path: e.target.value }); }}
                    placeholder="uv"
                  />
                </SettingsItem>
                <SettingsItem title="Python Version" wide>
                  <Input
                    value={runtimeForm.python_version}
                    onChange={(e) => { update({ python_version: e.target.value }); }}
                    placeholder="3.12"
                  />
                </SettingsItem>
                <SettingsItem title="Venv Directory" wide>
                  <Input
                    value={runtimeForm.python_venv_dir}
                    onChange={(e) => { update({ python_venv_dir: e.target.value }); }}
                    placeholder=".venv"
                  />
                </SettingsItem>
                <SettingsItem title="Working Directory" wide>
                  <Input
                    value={runtimeForm.python_working_dir}
                    onChange={(e) => { update({ python_working_dir: e.target.value }); }}
                    placeholder="(uses current dir)"
                  />
                </SettingsItem>
              </>
            )}
          </SettingsGroup>

          <SettingsGroup
            title="JavaScript Environment (bun)"
            description="When enabled, the Bun path is injected into the system prompt so the LLM uses the correct JS runtime"
          >
            <SettingsItem title="Enable JavaScript environment">
              <Checkbox
                checked={runtimeForm.bun_venv_enabled}
                onCheckedChange={(c) => { update({ bun_venv_enabled: c === true }); }}
              />
            </SettingsItem>
            {runtimeForm.bun_venv_enabled && (
              <>
                <SettingsItem title="bun Binary Path" wide>
                  <Input
                    value={runtimeForm.bun_path}
                    onChange={(e) => { update({ bun_path: e.target.value }); }}
                    placeholder="bun"
                  />
                </SettingsItem>
                <SettingsItem title="Bun Version" wide>
                  <Input
                    value={runtimeForm.bun_version}
                    onChange={(e) => { update({ bun_version: e.target.value }); }}
                    placeholder="latest"
                  />
                </SettingsItem>
                <SettingsItem title="Working Directory" wide>
                  <Input
                    value={runtimeForm.bun_working_dir}
                    onChange={(e) => { update({ bun_working_dir: e.target.value }); }}
                    placeholder="(uses current dir)"
                  />
                </SettingsItem>
              </>
            )}
          </SettingsGroup>
        </div>
      }
    />
  );
}
