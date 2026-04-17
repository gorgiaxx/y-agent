import { Input } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import { SettingsGroup, SettingsItem } from '../../ui';
import type { AgentDraft } from '../types';

interface ModelTabProps {
  draft: AgentDraft;
  providerOptions: Array<{ id: string; model: string }>;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function ModelTab({ draft, providerOptions, onChange }: ModelTabProps) {
  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="Provider">
        <SettingsItem title="Provider">
          <Select value={draft.provider_id || '__auto__'} onValueChange={(value) => onChange((prev) => ({ ...prev, provider_id: value === '__auto__' ? '' : value }))}>
            <SelectTrigger>
              <SelectValue placeholder="auto" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__auto__">auto</SelectItem>
              {providerOptions.map((provider) => (
                <SelectItem key={provider.id} value={provider.id}>{provider.model || provider.id}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Provider Tags" description="Comma-separated routing tags" wide>
          <Input value={draft.provider_tags} onChange={(event) => onChange((prev) => ({ ...prev, provider_tags: event.target.value }))} placeholder="general, code" />
        </SettingsItem>
        <SettingsItem title="Preferred Models" wide>
          <Input value={draft.preferred_models} onChange={(event) => onChange((prev) => ({ ...prev, preferred_models: event.target.value }))} placeholder="model-a, model-b" />
        </SettingsItem>
        <SettingsItem title="Fallback Models" wide>
          <Input value={draft.fallback_models} onChange={(event) => onChange((prev) => ({ ...prev, fallback_models: event.target.value }))} placeholder="model-c" />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Generation">
        <SettingsItem title="Plan Mode">
          <Select value={draft.plan_mode || '__default__'} onValueChange={(value) => onChange((prev) => ({ ...prev, plan_mode: value === '__default__' ? '' : value }))}>
            <SelectTrigger>
              <SelectValue placeholder="default" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__default__">default</SelectItem>
              <SelectItem value="fast">fast</SelectItem>
              <SelectItem value="auto">auto</SelectItem>
              <SelectItem value="plan">plan</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Thinking Effort">
          <Select value={draft.thinking_effort || '__default__'} onValueChange={(value) => onChange((prev) => ({ ...prev, thinking_effort: value === '__default__' ? '' : value }))}>
            <SelectTrigger>
              <SelectValue placeholder="default" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__default__">default</SelectItem>
              <SelectItem value="low">low</SelectItem>
              <SelectItem value="medium">medium</SelectItem>
              <SelectItem value="high">high</SelectItem>
              <SelectItem value="max">max</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Temperature">
          <Input numeric type="number" step={0.1} min={0} max={2} className="w-[100px]" value={draft.temperature} onChange={(event) => onChange((prev) => ({ ...prev, temperature: event.target.value }))} />
        </SettingsItem>
        <SettingsItem title="Top P">
          <Input numeric type="number" step={0.05} min={0} max={1} className="w-[100px]" value={draft.top_p} onChange={(event) => onChange((prev) => ({ ...prev, top_p: event.target.value }))} />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="MCP">
        <SettingsItem title="MCP Mode">
          <Select value={draft.mcp_mode || '__default__'} onValueChange={(value) => onChange((prev) => ({ ...prev, mcp_mode: value === '__default__' ? '' : value }))}>
            <SelectTrigger>
              <SelectValue placeholder="default" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__default__">default</SelectItem>
              <SelectItem value="auto">auto</SelectItem>
              <SelectItem value="manual">manual</SelectItem>
              <SelectItem value="disabled">disabled</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="MCP Servers" description="Manual mode server list" wide>
          <Input
            value={draft.mcp_servers.join(', ')}
            onChange={(event) => onChange((prev) => ({
              ...prev,
              mcp_servers: event.target.value.split(',').map((s) => s.trim()).filter(Boolean),
            }))}
            placeholder="server-a, server-b"
          />
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}
