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
    <div className="agent-editor-form-stack">
      <label className="agent-editor-checkbox-row">
        <Checkbox
          checked={draft.skills_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, skills_enabled: checked === true }))}
        />
        <span className="agent-editor-field-label">Enable skills</span>
      </label>
      <div className="agent-editor-checkbox-grid">
        {availableSkills.map((skill) => (
          <label
            key={skill}
            className={[
              'agent-editor-checkbox-card agent-editor-checkbox-card--center',
              draft.skills.includes(skill) ? 'agent-editor-checkbox-card--active' : '',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.skills.includes(skill)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, skills: toggleItem(prev.skills, skill) }))}
            />
            <div className="agent-editor-checkbox-card-title">{skill}</div>
          </label>
        ))}
      </div>
    </div>
  );
}
