import { Textarea } from '../../ui/Input';
import { Checkbox } from '../../ui/Checkbox';
import { SettingsGroup, SettingsItem } from '../../ui';
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
    <div className="settings-form-wrap">
      <SettingsGroup title="System Prompt">
        <SettingsItem title="Custom System Prompt" wide>
          <Textarea
            value={draft.system_prompt}
            onChange={(event) => onChange((prev) => ({ ...prev, system_prompt: event.target.value }))}
            rows={6}
            className="text-11px"
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Prompt Sections">
        <div className="settings-item--custom-body">
          <div className="agent-editor-checkbox-grid agent-editor-checkbox-grid--short">
            {promptSections.map((section) => (
              <label
                key={section.id}
                className={[
                  'agent-editor-checkbox-card',
                  draft.prompt_section_ids.includes(section.id) ? 'agent-editor-checkbox-card--active' : '',
                ].join(' ')}
              >
                <Checkbox
                  checked={draft.prompt_section_ids.includes(section.id)}
                  onCheckedChange={() => onChange((prev) => ({ ...prev, prompt_section_ids: toggleItem(prev.prompt_section_ids, section.id) }))}
                  className="mt-0.5"
                />
                <div className="agent-editor-checkbox-card-body">
                  <div className="agent-editor-checkbox-card-title">{section.id}</div>
                  <div className="agent-editor-checkbox-card-desc">{section.category}</div>
                </div>
              </label>
            ))}
          </div>
        </div>
      </SettingsGroup>
    </div>
  );
}
