import { PromptComposer } from '../../prompts/PromptComposer';
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
      <PromptComposer
        systemPrompt={draft.system_prompt}
        selectedSectionIds={draft.prompt_section_ids}
        promptSections={promptSections}
        mode={draft.mode}
        onSystemPromptChange={(value) => onChange((prev) => ({ ...prev, system_prompt: value }))}
        onSectionToggle={(id) => onChange((prev) => ({
          ...prev,
          prompt_section_ids: toggleItem(prev.prompt_section_ids, id),
        }))}
      />
    </div>
  );
}
