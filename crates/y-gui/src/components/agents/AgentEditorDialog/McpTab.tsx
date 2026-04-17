import { Checkbox, SettingsGroup, SettingsItem } from '../../ui';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../ui/Select';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

const MCP_MODE_OPTIONS = [
  { value: '', label: 'Use global default' },
  { value: 'enabled', label: 'Enabled' },
  { value: 'disabled', label: 'Disabled' },
  { value: 'per_tool', label: 'Per-tool approval' },
];

interface McpTabProps {
  draft: AgentDraft;
  mcpServers: { name: string; disabled: boolean }[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function McpTab({ draft, mcpServers, onChange }: McpTabProps) {
  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="MCP Mode">
        <SettingsItem title="MCP mode" description="Control how MCP servers interact with this agent">
          <Select
            value={draft.mcp_mode || '__default__'}
            onValueChange={(value) =>
              onChange((prev) => ({ ...prev, mcp_mode: value === '__default__' ? '' : value }))
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {MCP_MODE_OPTIONS.map((opt) => (
                <SelectItem key={opt.value || '__default__'} value={opt.value || '__default__'}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="MCP Servers">
        {mcpServers.length === 0 ? (
          <div className="pf-hint" style={{ padding: '8px 0' }}>
            No MCP servers configured. Add servers in Settings &rarr; MCP Servers.
          </div>
        ) : (
          mcpServers.map((server) => (
            <SettingsItem
              key={server.name}
              title={server.name}
              description={server.disabled ? '(disabled)' : undefined}
            >
              <Checkbox
                checked={draft.mcp_servers.includes(server.name)}
                onCheckedChange={() =>
                  onChange((prev) => ({
                    ...prev,
                    mcp_servers: toggleItem(prev.mcp_servers, server.name),
                  }))
                }
              />
            </SettingsItem>
          ))
        )}
      </SettingsGroup>
    </div>
  );
}
