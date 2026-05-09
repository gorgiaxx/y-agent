import { useCallback, useEffect, useMemo, useState, type Dispatch, type SetStateAction } from 'react';
import { Plus, Star, Trash2 } from 'lucide-react';

import { transport } from '../../lib';
import { STORAGE_KEYS } from '../../constants/storageKeys';
import type { UserPromptTemplate } from '../../types';
import type { PromptSectionInfo } from '../../hooks/useAgents';
import { PromptComposer, PromptPreviewPanel } from '../prompts/PromptComposer';
import { buildPromptPreview } from '../prompts/promptPreview';
import { Button, Input, SettingsGroup, SettingsItem, SubListLayout } from '../ui';

interface PromptTemplatesTabProps {
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
  setDirtyPromptTemplates: Dispatch<SetStateAction<boolean>>;
  registerSaveHandler: (handler: (() => Promise<void>) | null) => void;
}

const EMPTY_TEMPLATE: UserPromptTemplate = {
  id: '',
  name: '',
  description: null,
  system_prompt: '',
  prompt_section_ids: [],
};

export function PromptTemplatesTab({
  setToast,
  setDirtyPromptTemplates,
  registerSaveHandler,
}: PromptTemplatesTabProps) {
  const [templates, setTemplates] = useState<UserPromptTemplate[]>([]);
  const [promptSections, setPromptSections] = useState<PromptSectionInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [draft, setDraft] = useState<UserPromptTemplate>(EMPTY_TEMPLATE);
  const [loading, setLoading] = useState(true);
  const [defaultTemplateId, setDefaultTemplateId] = useState(() => (
    localStorage.getItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE) ?? ''
  ));

  const loadTemplates = useCallback(async () => {
    setLoading(true);
    try {
      const [templateList, sections] = await Promise.all([
        transport.invoke<UserPromptTemplate[]>('prompt_template_list'),
        transport.invoke<PromptSectionInfo[]>('agent_prompt_section_list'),
      ]);
      setTemplates(templateList);
      setPromptSections(sections);
      const nextActive = templateList[0]?.id ?? null;
      setActiveId(nextActive);
      setDraft(templateList.find((item) => item.id === nextActive) ?? EMPTY_TEMPLATE);
    } catch (e) {
      setToast({ message: `Failed to load prompt templates: ${e}`, type: 'error' });
    } finally {
      setLoading(false);
    }
  }, [setToast]);

  useEffect(() => {
    loadTemplates();
  }, [loadTemplates]);

  const isNew = !activeId;
  const activeTemplate = useMemo(
    () => templates.find((template) => template.id === activeId) ?? null,
    [activeId, templates],
  );
  const preview = useMemo(
    () => buildPromptPreview({
      systemPrompt: draft.system_prompt,
      selectedSectionIds: draft.prompt_section_ids,
      promptSections,
      mode: 'general',
    }),
    [draft.prompt_section_ids, draft.system_prompt, promptSections],
  );

  const updateDraft = useCallback((updater: (draft: UserPromptTemplate) => UserPromptTemplate) => {
    setDraft((prev) => updater(prev));
    setDirtyPromptTemplates(true);
  }, [setDirtyPromptTemplates]);

  const selectTemplate = (template: UserPromptTemplate) => {
    setActiveId(template.id);
    setDraft(template);
  };

  const createTemplate = () => {
    setActiveId(null);
    setDraft({
      ...EMPTY_TEMPLATE,
      name: 'New Template',
      id: 'new-template',
    });
    setDirtyPromptTemplates(true);
  };

  const saveTemplate = useCallback(async () => {
    const normalized = normalizeTemplate(draft);
    if (!normalized.id || !normalized.name) {
      throw new Error('Template id and name are required');
    }

    await transport.invoke('prompt_template_save', {
      id: normalized.id,
      template: normalized,
    });
    setTemplates((prev) => {
      const existing = prev.filter((item) => item.id !== normalized.id);
      return [...existing, normalized].sort((left, right) => left.name.localeCompare(right.name));
    });
    setActiveId(normalized.id);
    setDraft(normalized);
    setDirtyPromptTemplates(false);
  }, [draft, setDirtyPromptTemplates]);

  useEffect(() => {
    registerSaveHandler(saveTemplate);
    return () => registerSaveHandler(null);
  }, [registerSaveHandler, saveTemplate]);

  const deleteTemplate = async () => {
    if (!activeTemplate) return;
    try {
      await transport.invoke('prompt_template_delete', { id: activeTemplate.id });
      const next = templates.filter((template) => template.id !== activeTemplate.id);
      setTemplates(next);
      if (defaultTemplateId === activeTemplate.id) {
        localStorage.removeItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE);
        setDefaultTemplateId('');
      }
      setActiveId(next[0]?.id ?? null);
      setDraft(next[0] ?? EMPTY_TEMPLATE);
      setToast({ message: 'Prompt template deleted', type: 'success' });
    } catch (e) {
      setToast({ message: `Delete failed: ${e}`, type: 'error' });
    }
  };

  const makeDefault = (templateId: string) => {
    const id = templateId.trim();
    if (!id) return;
    if (defaultTemplateId === id) {
      localStorage.removeItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE);
      setDefaultTemplateId('');
      setToast({ message: 'Default prompt template cleared', type: 'success' });
      return;
    }
    localStorage.setItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE, id);
    setDefaultTemplateId(id);
    setToast({ message: 'Default prompt template updated', type: 'success' });
  };

  return (
    <div className="settings-section settings-section--fill">
      <div className="settings-header">
        <h3 className="section-title section-title--flush">Prompt Templates</h3>
      </div>

      {loading ? (
        <div className="section-loading">Loading...</div>
      ) : (
        <SubListLayout
          className="prompt-template-layout"
          sidebar={
            <>
              <div className="sub-list-actions">
                <button
                  type="button"
                  className="sub-list-item sub-list-item-add"
                  onClick={createTemplate}
                  title="Add template"
                >
                  <Plus size={13} />
                  <span>Add</span>
                </button>
                <Button
                  variant="icon"
                  size="sm"
                  onClick={deleteTemplate}
                  disabled={!activeTemplate}
                  title="Delete template"
                >
                  <Trash2 size={14} />
                </Button>
              </div>

              <div className="sub-list-items">
                {templates.map((template) => (
                  <button
                    key={template.id}
                    className={`sub-list-item ${activeId === template.id ? 'active' : ''}`}
                    onClick={() => selectTemplate(template)}
                  >
                    <span className="sub-list-item-label">{template.name}</span>
                    {defaultTemplateId === template.id && (
                      <span className="sub-list-item-meta">Default</span>
                    )}
                    <span
                      className={`sub-list-item-default ${defaultTemplateId === template.id ? 'active' : ''}`}
                      role="button"
                      tabIndex={0}
                      title={defaultTemplateId === template.id ? 'Clear default' : 'Set default'}
                      onClick={(e) => {
                        e.stopPropagation();
                        makeDefault(template.id);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault();
                          e.stopPropagation();
                          makeDefault(template.id);
                        }
                      }}
                    >
                      <Star size={11} />
                    </span>
                  </button>
                ))}
                {templates.length === 0 && (
                  <div className="settings-empty">No prompt templates</div>
                )}
              </div>
            </>
          }
        >

          <div className="prompt-template-detail">
            <div className="prompt-template-editor-column">
              <SettingsGroup title="Template">
                <SettingsItem title="Name">
                  <Input
                    value={draft.name}
                    onChange={(e) => updateDraft((prev) => ({
                      ...prev,
                      name: e.target.value,
                      id: isNew ? slugify(e.target.value) : prev.id,
                    }))}
                  />
                </SettingsItem>
                <SettingsItem title="ID">
                  <Input
                    value={draft.id}
                    onChange={(e) => updateDraft((prev) => ({
                      ...prev,
                      id: slugify(e.target.value),
                    }))}
                    disabled={!isNew}
                  />
                </SettingsItem>
              </SettingsGroup>

              <PromptComposer
                systemPrompt={draft.system_prompt}
                selectedSectionIds={draft.prompt_section_ids}
                promptSections={promptSections}
                mode="general"
                showPreview={false}
                onSystemPromptChange={(value) => updateDraft((prev) => ({
                  ...prev,
                  system_prompt: value,
                }))}
                onSelectedSectionIdsChange={(ids) => updateDraft((prev) => ({
                  ...prev,
                  prompt_section_ids: ids,
                }))}
              />
            </div>

            <PromptPreviewPanel preview={preview} className="prompt-template-preview-panel" />
          </div>
        </SubListLayout>
      )}
    </div>
  );
}

function normalizeTemplate(template: UserPromptTemplate): UserPromptTemplate {
  return {
    id: slugify(template.id),
    name: template.name.trim(),
    description: null,
    system_prompt: template.system_prompt.trim(),
    prompt_section_ids: [...new Set(template.prompt_section_ids)],
  };
}

function slugify(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}
