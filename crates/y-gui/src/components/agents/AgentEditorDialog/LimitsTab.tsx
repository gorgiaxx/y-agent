import { Input } from '../../ui/Input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../ui/Select';
import { SettingsGroup, SettingsItem } from '../../ui';
import type { AgentDraft } from '../types';

interface LimitsTabProps {
  draft: AgentDraft;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function LimitsTab({ draft, onChange }: LimitsTabProps) {
  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="Permissions">
        <SettingsItem title="Permission Mode">
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
        </SettingsItem>
        <SettingsItem title="Context Sharing">
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
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Limits">
        <SettingsItem title="Max Iterations" description="Also used as session turn cap for bound agent chats">
          <Input numeric type="number" min={1} className="w-[100px]" value={draft.max_iterations} onChange={(event) => onChange((prev) => ({ ...prev, max_iterations: event.target.value }))} />
        </SettingsItem>
        <SettingsItem title="Max Tool Calls">
          <Input numeric type="number" min={1} className="w-[100px]" value={draft.max_tool_calls} onChange={(event) => onChange((prev) => ({ ...prev, max_tool_calls: event.target.value }))} />
        </SettingsItem>
        <SettingsItem title="Timeout (seconds)">
          <Input numeric type="number" min={1} className="w-[100px]" value={draft.timeout_secs} onChange={(event) => onChange((prev) => ({ ...prev, timeout_secs: event.target.value }))} />
        </SettingsItem>
        <SettingsItem title="Max Context Tokens">
          <Input numeric type="number" min={1} className="w-[100px]" value={draft.max_context_tokens} onChange={(event) => onChange((prev) => ({ ...prev, max_context_tokens: event.target.value }))} />
        </SettingsItem>
        <SettingsItem title="Max Completion Tokens">
          <Input numeric type="number" min={1} className="w-[100px]" value={draft.max_completion_tokens} onChange={(event) => onChange((prev) => ({ ...prev, max_completion_tokens: event.target.value }))} />
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}
