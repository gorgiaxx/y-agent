// ---------------------------------------------------------------------------
// McpTab -- MCP servers list sidebar + McpServerTabPanel detail form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Plus, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { TagChipInput } from './TagChipInput';
import type { McpServerFormData } from './settingsTypes';
import { emptyMcpServer, jsonToMcpServers, mcpServersToJson } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, Button } from '../ui';

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
    <div className="sidetab-tab-form">
      {/* Row 0: Name + Transport */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Server Name</label>
          <Input
            value={server.name}
            onChange={(e) => update({ name: e.target.value })}
            placeholder="e.g. my-local-server"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Transport</label>
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
        </div>
      </div>

      {server.transport === 'stdio' ? (
        /* STDIO fields */
        <>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Command</label>
              <Input
                value={server.command}
                onChange={(e) => update({ command: e.target.value })}
                placeholder="e.g. node, python, npx"
              />
              <span className="pf-hint">Executable command to launch the MCP server process</span>
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Arguments</label>
              <TagChipInput
                tags={server.args}
                onChange={(next) => update({ args: next })}
              />
              <span className="pf-hint">Command-line arguments passed to the server process</span>
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Environment Variables</label>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                {Object.entries(server.env).map(([k, v], i) => (
                  <div key={i} style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
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
                    <span style={{ color: 'var(--text-secondary)' }}>=</span>
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
                  onClick={() => update({ env: { ...server.env, '': '' } })}
                >+ Add Variable</Button>
              </div>
            </div>
          </div>
        </>
      ) : (
        /* HTTP fields */
        <>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Server URL</label>
              <Input
                value={server.url}
                onChange={(e) => update({ url: e.target.value })}
                placeholder="https://your-server-url.com/mcp"
              />
              <span className="pf-hint">HTTP endpoint URL for the remote MCP server</span>
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Bearer Token</label>
              <Input
                type="password"
                value={server.bearer_token}
                onChange={(e) => update({ bearer_token: e.target.value })}
                placeholder="(optional) sent as Authorization: Bearer ..."
              />
              <span className="pf-hint">Optional OAuth bearer token for Authorization header</span>
            </div>
          </div>
          <div className="pf-row">
            <div className="pf-field pf-field-full">
              <label className="pf-label">Headers</label>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                {Object.entries(server.headers).map(([k, v], i) => (
                  <div key={i} style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
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
                    <span style={{ color: 'var(--text-secondary)' }}>:</span>
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
                  onClick={() => update({ headers: { ...server.headers, '': '' } })}
                >+ Add Header</Button>
              </div>
            </div>
          </div>
        </>
      )}

      {/* Always Allow */}
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Always Allow</label>
          <TagChipInput
            tags={server.alwaysAllow}
            onChange={(next) => update({ alwaysAllow: next })}
          />
          <span className="pf-hint">Tool names that are auto-approved without user confirmation</span>
        </div>
      </div>

      {/* Timeouts */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Startup timeout (s)</label>
          <Input
            type="number"
            min={1}
            value={server.startup_timeout_secs}
            onChange={(e) => update({ startup_timeout_secs: Number(e.target.value) || 30 })}
          />
          <span className="pf-hint">Initial connection / initialize handshake timeout</span>
        </div>
        <div className="pf-field">
          <label className="pf-label">Tool call timeout (s)</label>
          <Input
            type="number"
            min={1}
            value={server.tool_timeout_secs}
            onChange={(e) => update({ tool_timeout_secs: Number(e.target.value) || 120 })}
          />
          <span className="pf-hint">Per-tool-call timeout</span>
        </div>
      </div>

      {server.transport === 'stdio' && (
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">Working Directory</label>
            <Input
              value={server.cwd}
              onChange={(e) => update({ cwd: e.target.value })}
              placeholder="(optional) working directory for the server process"
            />
          </div>
        </div>
      )}

      {/* Disabled toggle */}
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">
            <Checkbox
              checked={server.disabled}
              onCheckedChange={(c) => update({ disabled: c === true })}
            />
            {' '}Disabled
          </label>
          <span className="pf-hint">When checked, this server will not be started or connected to</span>
        </div>
      </div>
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
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const json = await invoke<any>('mcp_config_get');
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
            <span className="settings-header-with-toggle">MCP Servers <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
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
        <span className="settings-header-with-toggle">MCP Servers <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
      </h3>
    </div>
    <div className="sub-list-layout">
      {/* Left sidebar list */}
      <div className="sub-list-sidebar">
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
      </div>

      {/* Right detail panel */}
      <div className="sub-list-detail">
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
      </div>
    </div>
    </>
  );
}
