import { Input, Textarea } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import { Checkbox } from '../../ui/Checkbox';
import { SettingsGroup, SettingsItem } from '../../ui';
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
    <div className="settings-form-wrap">
      <SettingsGroup title="Identity">
        {mode === 'create' && (
          <SettingsItem title="Template">
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
          </SettingsItem>
        )}
        <SettingsItem title="ID" wide>
          <Input
            value={draft.id}
            disabled={mode === 'edit'}
            onChange={(event) => onChange((prev) => ({ ...prev, id: event.target.value }))}
            placeholder="code-reviewer"
          />
        </SettingsItem>
        <SettingsItem title="Name" wide>
          <Input
            value={draft.name}
            onChange={(event) => onChange((prev) => ({ ...prev, name: event.target.value }))}
            placeholder="Code Reviewer"
          />
        </SettingsItem>
        <SettingsItem title="Description" wide>
          <Textarea
            value={draft.description}
            onChange={(event) => onChange((prev) => ({ ...prev, description: event.target.value }))}
            rows={2}
            className="text-11px"
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Behavior">
        <SettingsItem title="Mode">
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
        </SettingsItem>
        <SettingsItem title="Working Directory" wide>
          <Input
            value={draft.working_directory}
            onChange={(event) => onChange((prev) => ({ ...prev, working_directory: event.target.value }))}
            placeholder="/path/to/workspace"
          />
        </SettingsItem>
        <SettingsItem title="Show as selectable agent">
          <Checkbox
            checked={draft.user_callable}
            onCheckedChange={(checked) => onChange((prev) => ({ ...prev, user_callable: checked === true }))}
          />
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}
