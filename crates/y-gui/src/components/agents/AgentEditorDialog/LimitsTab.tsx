import { Input } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import type { AgentDraft } from '../types';

interface LimitsTabProps {
  draft: AgentDraft;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function LimitsTab({ draft, onChange }: LimitsTabProps) {
  return (
    <div className="grid grid-cols-2 gap-3">
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Permission Mode</span>
        <Select value={draft.permission_mode || '__default__'} onValueChange={(value) => onChange((prev) => ({ ...prev, permission_mode: value === '__default__' ? '' : value }))}>
          <SelectTrigger>
            <SelectValue placeholder="default" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__default__">default</SelectItem>
            <SelectItem value="plan">plan</SelectItem>
            <SelectItem value="accept_edits">accept_edits</SelectItem>
            <SelectItem value="bypass_permissions">bypass_permissions</SelectItem>
            <SelectItem value="dont_ask">dont_ask</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Max Iterations</span>
        <span className="text-10px text-[var(--text-muted)]">Also used as the session turn cap for bound agent chats.</span>
        <Input value={draft.max_iterations} onChange={(event) => onChange((prev) => ({ ...prev, max_iterations: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Max Tool Calls</span>
        <Input value={draft.max_tool_calls} onChange={(event) => onChange((prev) => ({ ...prev, max_tool_calls: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Timeout Seconds</span>
        <Input value={draft.timeout_secs} onChange={(event) => onChange((prev) => ({ ...prev, timeout_secs: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Context Sharing</span>
        <Select value={draft.context_sharing} onValueChange={(value) => onChange((prev) => ({ ...prev, context_sharing: value }))}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">none</SelectItem>
            <SelectItem value="summary">summary</SelectItem>
            <SelectItem value="filtered">filtered</SelectItem>
            <SelectItem value="full">full</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Max Context Tokens</span>
        <Input value={draft.max_context_tokens} onChange={(event) => onChange((prev) => ({ ...prev, max_context_tokens: event.target.value }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Max Completion Tokens</span>
        <Input value={draft.max_completion_tokens} onChange={(event) => onChange((prev) => ({ ...prev, max_completion_tokens: event.target.value }))} />
      </label>
    </div>
  );
}
