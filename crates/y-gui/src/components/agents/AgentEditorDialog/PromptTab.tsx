import { Checkbox } from '../../ui/Checkbox';
import { MonacoEditor } from '../../ui/MonacoEditor';
import { SettingsGroup } from '../../ui';
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
      <SettingsGroup title="System Prompt" bodyVariant="plain">
        <div className="settings-item--custom-body">
          <div className="prompt-editor-monaco" style={{ height: 260 }}>
            <MonacoEditor
              value={draft.system_prompt}
              onChange={(val) => onChange((prev) => ({ ...prev, system_prompt: val }))}
              language="markdown"
            />
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title="Prompt Sections" bodyVariant="plain">
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
