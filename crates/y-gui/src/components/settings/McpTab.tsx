// ---------------------------------------------------------------------------
// McpTab -- MCP servers list sidebar + McpServerTabPanel detail form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Plus, X } from 'lucide-react';
import { transport } from '../../lib';
import { TagChipInput } from './TagChipInput';
import type { McpServerFormData } from './settingsTypes';
import { emptyMcpServer, jsonToMcpServers, mcpServersToJson } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, Button, SettingsGroup, SettingsItem, SubListLayout } from '../ui';

// ---------------------------------------------------------------------------
// McpServerTabPanel -- form for a single MCP server (shown in tab view)
// ---------------------------------------------------------------------------

function McpServerTabPanel({
  server,
  index,
  onChange,
}: {
  server: McpServerFormData;
  index: number;
  onChange: (index: number, updated: McpServerFormData) => void;
}) {
  const update = (patch: Partial<McpServerFormData>) => {
    onChange(index, { ...server, ...patch });
  };

  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="Identity">
        <SettingsItem title="Server Name" wide>
          <Input
            value={server.name}
            onChange={(e) => update({ name: e.target.value })}
            placeholder="e.g. my-local-server"
          />
        </SettingsItem>
        <SettingsItem title="Transport">
          <Select
            value={server.transport}
            onValueChange={(val) => update({ transport: val as 'stdio' | 'http' })}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select transport" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stdio">STDIO (Local)</SelectItem>
              <SelectItem value="http">HTTP (Remote)</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Disabled" description="Server will not be started or connected to">
          <Checkbox
            checked={server.disabled}
            onCheckedChange={(c) => update({ disabled: c === true })}
          />
        </SettingsItem>
      </SettingsGroup>

      {server.transport === 'stdio' ? (
        <SettingsGroup title="STDIO Configuration" bodyVariant="plain">
          <SettingsItem title="Command" description="Executable to launch the MCP server process" wide>
            <Input
              value={server.command}
              onChange={(e) => update({ command: e.target.value })}
              placeholder="e.g. node, python, npx"
            />
          </SettingsItem>
          <SettingsItem title="Arguments" description="Command-line arguments passed to the server" wide>
            <TagChipInput
              tags={server.args}
              onChange={(next) => update({ args: next })}
            />
          </SettingsItem>
          <SettingsItem title="Working Directory" wide>
            <Input
              value={server.cwd}
              onChange={(e) => update({ cwd: e.target.value })}
              placeholder="(optional) working directory"
            />
          </SettingsItem>
          <div className="settings-item--custom-body">
            <div className="settings-item-title" style={{ marginBottom: '6px' }}>Environment Variables</div>
            <div className="pf-kv-list">
              {Object.entries(server.env).map(([k, v], i) => (
                <div key={i} className="pf-kv-row">
                  <Input
                    style={{ flex: 1 }}
                    value={k}
                    onChange={(e) => {
                      const entries = Object.entries(server.env);
                      entries[i] = [e.target.value, v];
                      update({ env: Object.fromEntries(entries) });
                    }}
                    placeholder="KEY"
                  />
                  <span className="pf-kv-sep">=</span>
                  <Input
                    style={{ flex: 2 }}
                    value={v}
                    onChange={(e) => {
                      const newEnv = { ...server.env };
                      newEnv[k] = e.target.value;
                      update({ env: newEnv });
                    }}
                    placeholder="value"
                  />
                  <Button
                    variant="icon"
                    size="sm"
                    title="Remove"
                    onClick={() => {
                      const newEnv = { ...server.env };
                      delete newEnv[k];
                      update({ env: newEnv });
                    }}
                  ><X size={12} /></Button>
                </div>
              ))}
              <Button
                variant="ghost"
                size="sm"
                className="pf-kv-add"
                onClick={() => update({ env: { ...server.env, '': '' } })}
              >+ Add Variable</Button>
            </div>
          </div>
        </SettingsGroup>
      ) : (
        <SettingsGroup title="HTTP Configuration" bodyVariant="plain">
          <SettingsItem title="Server URL" description="HTTP endpoint for the remote MCP server" wide>
            <Input
              value={server.url}
              onChange={(e) => update({ url: e.target.value })}
              placeholder="https://your-server-url.com/mcp"
            />
          </SettingsItem>
          <SettingsItem title="Bearer Token" description="Optional OAuth bearer token" wide>
            <Input
              type="password"
              value={server.bearer_token}
              onChange={(e) => update({ bearer_token: e.target.value })}
              placeholder="(optional) Authorization: Bearer ..."
            />
          </SettingsItem>
          <div className="settings-item--custom-body">
            <div className="settings-item-title" style={{ marginBottom: '6px' }}>Headers</div>
            <div className="pf-kv-list">
              {Object.entries(server.headers).map(([k, v], i) => (
                <div key={i} className="pf-kv-row">
                  <Input
                    style={{ flex: 1 }}
                    value={k}
                    onChange={(e) => {
                      const entries = Object.entries(server.headers);
                      entries[i] = [e.target.value, v];
                      update({ headers: Object.fromEntries(entries) });
                    }}
                    placeholder="Header-Name"
                  />
                  <span className="pf-kv-sep">:</span>
                  <Input
                    style={{ flex: 2 }}
                    value={v}
                    onChange={(e) => {
                      const newHeaders = { ...server.headers };
                      newHeaders[k] = e.target.value;
                      update({ headers: newHeaders });
                    }}
                    placeholder="value"
                  />
                  <Button
                    variant="icon"
                    size="sm"
                    title="Remove"
                    onClick={() => {
                      const newHeaders = { ...server.headers };
                      delete newHeaders[k];
                      update({ headers: newHeaders });
                    }}
                  ><X size={12} /></Button>
                </div>
              ))}
              <Button
                variant="ghost"
                size="sm"
                className="pf-kv-add"
                onClick={() => update({ headers: { ...server.headers, '': '' } })}
              >+ Add Header</Button>
            </div>
          </div>
        </SettingsGroup>
      )}

      <SettingsGroup title="Permissions">
        <SettingsItem title="Always Allow" description="Tool names auto-approved without confirmation" wide>
          <TagChipInput
            tags={server.alwaysAllow}
            onChange={(next) => update({ alwaysAllow: next })}
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Timeouts">
        <SettingsItem title="Startup Timeout (s)" description="Initial connection handshake timeout">
          <Input
            numeric
            type="number"
            min={1}
            className="w-[100px]"
            value={server.startup_timeout_secs}
            onChange={(e) => update({ startup_timeout_secs: Number(e.target.value) || 30 })}
          />
        </SettingsItem>
        <SettingsItem title="Tool Call Timeout (s)" description="Per-tool-call timeout">
          <Input
            numeric
            type="number"
            min={1}
            className="w-[100px]"
            value={server.tool_timeout_secs}
            onChange={(e) => update({ tool_timeout_secs: Number(e.target.value) || 120 })}
          />
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}

// ---------------------------------------------------------------------------
// McpTab -- the full MCP tab with sidebar list + detail panel
// ---------------------------------------------------------------------------

interface McpTabProps {
  mcpServersList: McpServerFormData[];
  setMcpServersList: React.Dispatch<React.SetStateAction<McpServerFormData[]>>;
  setDirtyMcp: React.Dispatch<React.SetStateAction<boolean>>;
}

export function McpTab({
  mcpServersList,
  setMcpServersList,
  setDirtyMcp,
}: McpTabProps) {
  const [loading, setLoading] = useState(false);
  const [activeMcpTab, setActiveMcpTab] = useState(0);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');

  const loadMcpServers = useCallback(async () => {
    setLoading(true);
    try {
      const json = await transport.invoke<Record<string, unknown>>('mcp_config_get');
      setMcpServersList(jsonToMcpServers(json));
    } catch {
      // Use empty list if file not found.
    } finally {
      setLoading(false);
    }
  }, [setMcpServersList]);

  useEffect(() => {
    loadMcpServers();
  }, [loadMcpServers]);

  const handleMcpServerChange = useCallback((index: number, updated: McpServerFormData) => {
    setMcpServersList((prev) => prev.map((s, i) => (i === index ? updated : s)));
    setDirtyMcp(true);
  }, [setMcpServersList, setDirtyMcp]);

  const handleMcpServerRemove = useCallback((index: number) => {
    setMcpServersList((prev) => {
      const next = prev.filter((_, i) => i !== index);
      return next;
    });
    setActiveMcpTab((prev) => Math.max(0, prev > index ? prev - 1 : Math.min(prev, mcpServersList.length - 2)));
    setDirtyMcp(true);
  }, [mcpServersList.length, setMcpServersList, setDirtyMcp]);

  const handleMcpServerAdd = useCallback(() => {
    setMcpServersList((prev) => {
      setActiveMcpTab(prev.length);
      return [...prev, emptyMcpServer()];
    });
    setDirtyMcp(true);
  }, [setMcpServersList, setDirtyMcp]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  const handleToggleRaw = (next: boolean) => {
    if (next) {
      // Serialize current MCP list to formatted JSON
      setRawContent(JSON.stringify(mcpServersToJson(mcpServersList), null, 2));
    }
    setRawMode(next);
  };

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            // MCP uses JSON; mark dirty via setDirtyMcp and we'll handle
            // the raw JSON save in unified save.
            setDirtyMcp(true);
          }}
          placeholder="No MCP servers configured. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
    <div className="settings-header">
      <h3 className="section-title section-title--flush">
        <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <SubListLayout
      sidebar={
        <>
          <div className="sub-list-items">
            {mcpServersList.map((s, i) => (
              <button
                key={i}
                className={`sub-list-item ${activeMcpTab === i ? 'active' : ''}`}
                onClick={() => setActiveMcpTab(i)}
              >
                <span className="sub-list-item-label">{s.name || `Server ${i + 1}`}</span>
                {s.disabled && <span style={{ fontSize: '9px', color: 'var(--text-muted)', marginLeft: '2px' }}>OFF</span>}
                <span
                  className="sub-list-item-close"
                  role="button"
                  tabIndex={0}
                  title="Remove server"
                  onClick={(e) => { e.stopPropagation(); handleMcpServerRemove(i); }}
                  onKeyDown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleMcpServerRemove(i); } }}
                >
                  <X size={11} />
                </span>
              </button>
            ))}
          </div>
          <button
            className="sub-list-item sub-list-item-add"
            onClick={handleMcpServerAdd}
            title="Add MCP server"
          >
            <Plus size={13} />
            <span>Add</span>
          </button>
        </>
      }
    >
      {mcpServersList.length === 0 ? (
        <div className="settings-empty">
          No MCP servers configured. Click + to add one.
        </div>
      ) : (
        <McpServerTabPanel
          key={activeMcpTab}
          server={mcpServersList[activeMcpTab] ?? mcpServersList[0]}
          index={activeMcpTab < mcpServersList.length ? activeMcpTab : 0}
          onChange={handleMcpServerChange}
        />
      )}
    </SubListLayout>
    </>
  );
}
