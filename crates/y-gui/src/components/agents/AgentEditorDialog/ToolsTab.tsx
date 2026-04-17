import { Checkbox } from '../../ui/Checkbox';
import type { AgentToolInfo } from '../../../hooks/useAgents';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

interface ToolsTabProps {
  draft: AgentDraft;
  tools: AgentToolInfo[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function ToolsTab({ draft, tools, onChange }: ToolsTabProps) {
  return (
    <div className="agent-editor-form-stack">
      <label className="agent-editor-checkbox-row">
        <Checkbox
          checked={draft.toolcall_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, toolcall_enabled: checked === true }))}
        />
        <span className="agent-editor-field-label">Enable tool calls</span>
      </label>
      <div className="agent-editor-checkbox-grid">
        {tools.map((tool) => (
          <label
            key={tool.name}
            className={[
              'agent-editor-checkbox-card',
              draft.allowed_tools.includes(tool.name) ? 'agent-editor-checkbox-card--active' : '',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.allowed_tools.includes(tool.name)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, allowed_tools: toggleItem(prev.allowed_tools, tool.name) }))}
              className="mt-0.5"
            />
            <div className="agent-editor-checkbox-card-body">
              <div className="agent-editor-checkbox-card-title">{tool.name}</div>
              <div className="agent-editor-checkbox-card-desc">{tool.description}</div>
            </div>
          </label>
        ))}
      </div>
    </div>
  );
}
