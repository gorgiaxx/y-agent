import { Checkbox } from '../../ui/Checkbox';
import { SettingsGroup, SettingsItem } from '../../ui';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

interface SkillsTabProps {
  draft: AgentDraft;
  availableSkills: string[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function SkillsTab({ draft, availableSkills, onChange }: SkillsTabProps) {
  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="Skills">
        <SettingsItem title="Enable skills">
          <Checkbox
            checked={draft.skills_enabled}
            onCheckedChange={(checked) => onChange((prev) => ({ ...prev, skills_enabled: checked === true }))}
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Available Skills">
        <div className="settings-item--custom-body">
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
      </SettingsGroup>
    </div>
  );
}
