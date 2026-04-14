import { Input, Textarea } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import { Checkbox } from '../../ui/Checkbox';
import { AGENT_ICON_OPTIONS } from '../agentDisplay';
import type { AgentDraft } from '../types';

interface GeneralTabProps {
  mode: 'create' | 'edit';
  draft: AgentDraft;
  agents: Array<{ id: string; name: string }>;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
  onApplyTemplate: (id: string) => void;
}

export function GeneralTab({ mode, draft, agents, onChange, onApplyTemplate }: GeneralTabProps) {
  return (
    <div className="grid grid-cols-2 gap-3">
      {mode === 'create' && (
        <label className="col-span-2 flex flex-col gap-1.5">
          <span className="text-11px text-[var(--text-secondary)]">Template</span>
          <Select value="__none__" onValueChange={(value) => value !== '__none__' && onApplyTemplate(value)}>
            <SelectTrigger>
              <SelectValue placeholder="Start from scratch" />
            </SelectTrigger>
            <SelectContent>
              {agents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
      )}
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">ID</span>
        <Input
          value={draft.id}
          disabled={mode === 'edit'}
          onChange={(event) => onChange((prev) => ({ ...prev, id: event.target.value }))}
          placeholder="code-reviewer"
        />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Name</span>
        <Input
          value={draft.name}
          onChange={(event) => onChange((prev) => ({ ...prev, name: event.target.value }))}
          placeholder="Code Reviewer"
        />
      </label>
      <div className="col-span-2 flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Icon Token</span>
        <div className="flex items-center gap-2">
          <Input
            value={draft.icon}
            onChange={(event) => onChange((prev) => ({ ...prev, icon: event.target.value }))}
            placeholder="bot"
            className="flex-1"
          />
        </div>
        <span className="text-10px text-[var(--text-muted)]">
          Use a token such as <code>bot</code> or <code>knowledge</code> to keep the UI icon system consistent.
        </span>
        <div className="flex flex-wrap gap-1.5">
          {AGENT_ICON_OPTIONS.map((option) => (
            <button
              key={option.label}
              className={[
                'flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-solid',
                'text-10px transition-colors duration-150',
                draft.icon === option.value
                  ? 'border-[var(--accent)] bg-[var(--accent-subtle)] text-[var(--accent)]'
                  : 'border-[var(--border)] text-[var(--text-muted)] hover:border-[var(--border-focus)] hover:text-[var(--text-secondary)]',
              ].join(' ')}
              onClick={() => onChange((prev) => ({ ...prev, icon: option.value }))}
            >
              <option.Icon size={12} />
              <span>{option.label}</span>
            </button>
          ))}
        </div>
      </div>
      <label className="col-span-2 flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Description</span>
        <Textarea
          value={draft.description}
          onChange={(event) => onChange((prev) => ({ ...prev, description: event.target.value }))}
          rows={2}
          className="text-11px"
        />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Mode</span>
        <Select value={draft.mode} onValueChange={(value) => onChange((prev) => ({ ...prev, mode: value }))}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="general">general</SelectItem>
            <SelectItem value="build">build</SelectItem>
            <SelectItem value="plan">plan</SelectItem>
            <SelectItem value="explore">explore</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Working Directory</span>
        <Input
          value={draft.working_directory}
          onChange={(event) => onChange((prev) => ({ ...prev, working_directory: event.target.value }))}
          placeholder="/path/to/workspace"
        />
      </label>
      <label className="col-span-2 flex items-center gap-2 cursor-pointer">
        <Checkbox
          checked={draft.user_callable}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, user_callable: checked === true }))}
        />
        <span className="text-11px text-[var(--text-secondary)]">Show as selectable agent</span>
      </label>
    </div>
  );
}
