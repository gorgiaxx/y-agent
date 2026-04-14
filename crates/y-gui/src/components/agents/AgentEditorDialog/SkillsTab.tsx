import { Checkbox } from '../../ui/Checkbox';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

interface SkillsTabProps {
  draft: AgentDraft;
  availableSkills: string[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function SkillsTab({ draft, availableSkills, onChange }: SkillsTabProps) {
  return (
    <div className="flex flex-col gap-3">
      <label className="flex items-center gap-2 cursor-pointer">
        <Checkbox
          checked={draft.skills_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, skills_enabled: checked === true }))}
        />
        <span className="text-11px text-[var(--text-secondary)]">Enable skills</span>
      </label>
      <div className="grid grid-cols-2 gap-2 max-h-[320px] overflow-y-auto">
        {availableSkills.map((skill) => (
          <label
            key={skill}
            className={[
              'flex items-center gap-2 p-2 rounded-[var(--radius-sm)] border border-solid cursor-pointer',
              'transition-colors duration-150',
              draft.skills.includes(skill)
                ? 'border-[var(--accent)] bg-[var(--accent-subtle)]'
                : 'border-[var(--border)] hover:border-[var(--border-focus)]',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.skills.includes(skill)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, skills: toggleItem(prev.skills, skill) }))}
            />
            <div className="text-11px font-500 text-[var(--text-primary)] truncate">{skill}</div>
          </label>
        ))}
      </div>
    </div>
  );
}
