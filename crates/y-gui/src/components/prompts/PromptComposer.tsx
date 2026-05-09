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
  showPreview?: boolean;
  onSystemPromptChange: (value: string) => void;
  onSelectedSectionIdsChange: (ids: string[]) => void;
}

interface PromptPreviewPanelProps {
  preview: string;
  className?: string;
}

export function PromptPreviewPanel({ preview, className }: PromptPreviewPanelProps) {
  return (
    <div className={['prompt-composer-right', className ?? ''].filter(Boolean).join(' ')}>
      <SettingsGroup title="Final Prompt Preview" bodyVariant="plain">
        <div className="settings-item--custom-body">
          <div className="prompt-composer-preview-editor">
            <MonacoEditor
              value={preview || 'Default prompt sections'}
              language="markdown"
              readOnly
            />
          </div>
        </div>
      </SettingsGroup>
    </div>
  );
}

export function PromptComposer({
  systemPrompt,
  selectedSectionIds,
  promptSections,
  mode = 'general',
  showPreview = true,
  onSystemPromptChange,
  onSelectedSectionIdsChange,
}: PromptComposerProps) {
  const allSectionIds = useMemo(
    () => promptSections.map((section) => section.id),
    [promptSections],
  );
  const effectiveSelectedSectionIds = useMemo(
    () => (selectedSectionIds.length > 0 ? selectedSectionIds : allSectionIds),
    [allSectionIds, selectedSectionIds],
  );
  const selected = useMemo(() => new Set(effectiveSelectedSectionIds), [effectiveSelectedSectionIds]);
  const preview = useMemo(
    () => buildPromptPreview({ systemPrompt, selectedSectionIds, promptSections, mode }),
    [mode, promptSections, selectedSectionIds, systemPrompt],
  );

  const handleSectionToggle = (id: string) => {
    if (selectedSectionIds.length === 0) {
      onSelectedSectionIdsChange(allSectionIds.filter((sectionId) => sectionId !== id));
      return;
    }

    const next = selected.has(id)
      ? effectiveSelectedSectionIds.filter((sectionId) => sectionId !== id)
      : [...effectiveSelectedSectionIds, id];

    onSelectedSectionIdsChange(next);
  };

  return (
    <div
      className={[
        'prompt-composer',
        showPreview ? '' : 'prompt-composer--inputs-only',
      ].filter(Boolean).join(' ')}
    >
      <div className="prompt-composer-left">
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
            <div className="agent-editor-checkbox-grid">
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
                      onCheckedChange={() => handleSectionToggle(section.id)}
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
      </div>

      {showPreview && <PromptPreviewPanel preview={preview} />}
    </div>
  );
}
