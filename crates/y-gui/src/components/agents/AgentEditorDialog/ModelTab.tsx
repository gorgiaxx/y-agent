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
    <div className="grid grid-cols-2 gap-3">
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Provider</span>
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
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Plan Mode</span>
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
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Thinking Effort</span>
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
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Temperature</span>
        <Input value={draft.temperature} onChange={(event) => onChange((prev) => ({ ...prev, temperature: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Top P</span>
        <Input value={draft.top_p} onChange={(event) => onChange((prev) => ({ ...prev, top_p: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Preferred Models</span>
        <Input value={draft.preferred_models} onChange={(event) => onChange((prev) => ({ ...prev, preferred_models: event.target.value }))} placeholder="model-a, model-b" />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Fallback Models</span>
        <Input value={draft.fallback_models} onChange={(event) => onChange((prev) => ({ ...prev, fallback_models: event.target.value }))} placeholder="model-c" />
      </label>
      <label className="col-span-2 flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Provider Tags</span>
        <Input value={draft.provider_tags} onChange={(event) => onChange((prev) => ({ ...prev, provider_tags: event.target.value }))} placeholder="general, code" />
      </label>
    </div>
  );
}
