import { Input } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import type { AgentDraft } from '../types';

interface ModelTabProps {
  draft: AgentDraft;
  providerOptions: Array<{ id: string; model: string }>;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function ModelTab({ draft, providerOptions, onChange }: ModelTabProps) {
  return (
    <div className="agent-editor-form-grid">
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Provider</span>
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
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Plan Mode</span>
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
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Thinking Effort</span>
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
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Temperature</span>
        <Input value={draft.temperature} onChange={(event) => onChange((prev) => ({ ...prev, temperature: event.target.value }))} />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Top P</span>
        <Input value={draft.top_p} onChange={(event) => onChange((prev) => ({ ...prev, top_p: event.target.value }))} />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Preferred Models</span>
        <Input value={draft.preferred_models} onChange={(event) => onChange((prev) => ({ ...prev, preferred_models: event.target.value }))} placeholder="model-a, model-b" />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Fallback Models</span>
        <Input value={draft.fallback_models} onChange={(event) => onChange((prev) => ({ ...prev, fallback_models: event.target.value }))} placeholder="model-c" />
      </label>
      <label className="agent-editor-field agent-editor-field--full">
        <span className="agent-editor-field-label">Provider Tags</span>
        <Input value={draft.provider_tags} onChange={(event) => onChange((prev) => ({ ...prev, provider_tags: event.target.value }))} placeholder="general, code" />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">MCP Mode</span>
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
      </label>
      <label className="agent-editor-field agent-editor-field--full">
        <span className="agent-editor-field-label">MCP Servers (manual mode)</span>
        <Input
          value={draft.mcp_servers.join(', ')}
          onChange={(event) => onChange((prev) => ({
            ...prev,
            mcp_servers: event.target.value.split(',').map((s) => s.trim()).filter(Boolean),
          }))}
          placeholder="server-a, server-b"
        />
      </label>
    </div>
  );
}
