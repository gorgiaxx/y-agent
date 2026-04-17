import { Input, Textarea } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import { Checkbox } from '../../ui/Checkbox';
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
    <div className="agent-editor-form-grid">
      {mode === 'create' && (
        <label className="agent-editor-field agent-editor-field--full">
          <span className="agent-editor-field-label">Template</span>
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
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">ID</span>
        <Input
          value={draft.id}
          disabled={mode === 'edit'}
          onChange={(event) => onChange((prev) => ({ ...prev, id: event.target.value }))}
          placeholder="code-reviewer"
        />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Name</span>
        <Input
          value={draft.name}
          onChange={(event) => onChange((prev) => ({ ...prev, name: event.target.value }))}
          placeholder="Code Reviewer"
        />
      </label>
      <label className="agent-editor-field agent-editor-field--full">
        <span className="agent-editor-field-label">Description</span>
        <Textarea
          value={draft.description}
          onChange={(event) => onChange((prev) => ({ ...prev, description: event.target.value }))}
          rows={2}
          className="text-11px"
        />
      </label>
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Mode</span>
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
      <label className="agent-editor-field">
        <span className="agent-editor-field-label">Working Directory</span>
        <Input
          value={draft.working_directory}
          onChange={(event) => onChange((prev) => ({ ...prev, working_directory: event.target.value }))}
          placeholder="/path/to/workspace"
        />
      </label>
      <label className="agent-editor-checkbox-row agent-editor-field--full">
        <Checkbox
          checked={draft.user_callable}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, user_callable: checked === true }))}
        />
        <span className="agent-editor-field-label">Show as selectable agent</span>
      </label>
    </div>
  );
}
