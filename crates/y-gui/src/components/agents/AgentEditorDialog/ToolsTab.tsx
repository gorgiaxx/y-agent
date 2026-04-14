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
    <div className="flex flex-col gap-3">
      <label className="flex items-center gap-2 cursor-pointer">
        <Checkbox
          checked={draft.toolcall_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, toolcall_enabled: checked === true }))}
        />
        <span className="text-11px text-[var(--text-secondary)]">Enable tool calls</span>
      </label>
      <div className="grid grid-cols-2 gap-2 max-h-[320px] overflow-y-auto">
        {tools.map((tool) => (
          <label
            key={tool.name}
            className={[
              'flex items-start gap-2 p-2 rounded-[var(--radius-sm)] border border-solid cursor-pointer',
              'transition-colors duration-150',
              draft.allowed_tools.includes(tool.name)
                ? 'border-[var(--accent)] bg-[var(--accent-subtle)]'
                : 'border-[var(--border)] hover:border-[var(--border-focus)]',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.allowed_tools.includes(tool.name)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, allowed_tools: toggleItem(prev.allowed_tools, tool.name) }))}
              className="mt-0.5"
            />
            <div className="min-w-0 flex-1">
              <div className="text-11px font-500 text-[var(--text-primary)] truncate">{tool.name}</div>
              <div className="text-10px text-[var(--text-muted)] line-clamp-2 mt-0.5">{tool.description}</div>
            </div>
          </label>
        ))}
      </div>
    </div>
  );
}
