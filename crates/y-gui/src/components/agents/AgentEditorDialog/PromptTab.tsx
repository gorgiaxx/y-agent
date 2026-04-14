import { Textarea } from '../../ui/Input';
import { Checkbox } from '../../ui/Checkbox';
import type { PromptSectionInfo } from '../../../hooks/useAgents';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

interface PromptTabProps {
  draft: AgentDraft;
  promptSections: PromptSectionInfo[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function PromptTab({ draft, promptSections, onChange }: PromptTabProps) {
  return (
    <div className="flex flex-col gap-3">
      <label className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Custom System Prompt</span>
        <Textarea
          value={draft.system_prompt}
          onChange={(event) => onChange((prev) => ({ ...prev, system_prompt: event.target.value }))}
          rows={6}
          className="text-11px"
        />
      </label>
      <div className="flex flex-col gap-1.5">
        <span className="text-11px text-[var(--text-secondary)]">Prompt Sections</span>
        <div className="grid grid-cols-2 gap-2 max-h-[200px] overflow-y-auto">
          {promptSections.map((section) => (
            <label
              key={section.id}
              className={[
                'flex items-start gap-2 p-2 rounded-[var(--radius-sm)] border border-solid cursor-pointer',
                'transition-colors duration-150',
                draft.prompt_section_ids.includes(section.id)
                  ? 'border-[var(--accent)] bg-[var(--accent-subtle)]'
                  : 'border-[var(--border)] hover:border-[var(--border-focus)]',
              ].join(' ')}
            >
              <Checkbox
                checked={draft.prompt_section_ids.includes(section.id)}
                onCheckedChange={() => onChange((prev) => ({ ...prev, prompt_section_ids: toggleItem(prev.prompt_section_ids, section.id) }))}
                className="mt-0.5"
              />
              <div className="min-w-0 flex-1">
                <div className="text-11px font-500 text-[var(--text-primary)] truncate">{section.id}</div>
                <div className="text-10px text-[var(--text-muted)] mt-0.5">{section.category}</div>
              </div>
            </label>
          ))}
        </div>
      </div>
    </div>
  );
}
