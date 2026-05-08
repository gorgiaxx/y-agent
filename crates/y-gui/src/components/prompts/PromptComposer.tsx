import { useMemo } from 'react';

import { Checkbox } from '../ui/Checkbox';
import { MonacoEditor } from '../ui/MonacoEditor';
import { SettingsGroup } from '../ui/SettingsGroup';
import {
  buildPromptPreview,
  type PromptSectionForComposer,
} from './promptPreview';

import './PromptComposer.css';

export type { PromptSectionForComposer };

interface PromptComposerProps {
  systemPrompt: string;
  selectedSectionIds: string[];
  promptSections: PromptSectionForComposer[];
  mode?: string;
  onSystemPromptChange: (value: string) => void;
  onSectionToggle: (id: string) => void;
}

export function PromptComposer({
  systemPrompt,
  selectedSectionIds,
  promptSections,
  mode = 'general',
  onSystemPromptChange,
  onSectionToggle,
}: PromptComposerProps) {
  const selected = useMemo(() => new Set(selectedSectionIds), [selectedSectionIds]);
  const preview = useMemo(
    () => buildPromptPreview({ systemPrompt, selectedSectionIds, promptSections, mode }),
    [mode, promptSections, selectedSectionIds, systemPrompt],
  );

  return (
    <div className="prompt-composer">
      <SettingsGroup title="System Prompt" bodyVariant="plain">
        <div className="settings-item--custom-body">
          <div className="prompt-composer-editor">
            <MonacoEditor
              value={systemPrompt}
              onChange={onSystemPromptChange}
              language="markdown"
              placeholder="Enter custom system prompt..."
            />
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title="Prompt Sections" bodyVariant="plain">
        <div className="settings-item--custom-body">
          <div className="agent-editor-checkbox-grid agent-editor-checkbox-grid--short">
            {promptSections.map((section) => {
              const isActive = selected.has(section.id);
              return (
                <label
                  key={section.id}
                  className={[
                    'agent-editor-checkbox-card',
                    isActive ? 'agent-editor-checkbox-card--active' : '',
                  ].join(' ')}
                >
                  <Checkbox
                    checked={isActive}
                    onCheckedChange={() => onSectionToggle(section.id)}
                    className="mt-0.5"
                  />
                  <div className="agent-editor-checkbox-card-body">
                    <div className="agent-editor-checkbox-card-title">{section.id}</div>
                    <div className="agent-editor-checkbox-card-desc">{section.category}</div>
                  </div>
                </label>
              );
            })}
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title="Final Prompt Preview" bodyVariant="plain">
        <div className="settings-item--custom-body">
          <pre className="prompt-composer-preview">{preview || 'Default prompt sections'}</pre>
        </div>
      </SettingsGroup>
    </div>
  );
}
