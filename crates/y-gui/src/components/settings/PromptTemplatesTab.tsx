import { useCallback, useEffect, useMemo, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';

import { transport } from '../../lib';
import { STORAGE_KEYS } from '../../constants/storageKeys';
import type { UserPromptTemplate } from '../../types';
import type { PromptSectionInfo } from '../../hooks/useAgents';
import { PromptComposer } from '../prompts/PromptComposer';
import { Button, Input, SettingsGroup, SettingsItem, Textarea } from '../ui';

interface PromptTemplatesTabProps {
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
}

const EMPTY_TEMPLATE: UserPromptTemplate = {
  id: '',
  name: '',
  description: '',
  system_prompt: '',
  prompt_section_ids: [],
};

export function PromptTemplatesTab({ setToast }: PromptTemplatesTabProps) {
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
      const nextActive = activeId ?? templateList[0]?.id ?? null;
      setActiveId(nextActive);
      setDraft(templateList.find((item) => item.id === nextActive) ?? EMPTY_TEMPLATE);
    } catch (e) {
      setToast({ message: `Failed to load prompt templates: ${e}`, type: 'error' });
    } finally {
      setLoading(false);
    }
  }, [activeId, setToast]);

  useEffect(() => {
    loadTemplates();
  }, [loadTemplates]);

  const isNew = !activeId;
  const activeTemplate = useMemo(
    () => templates.find((template) => template.id === activeId) ?? null,
    [activeId, templates],
  );

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
  };

  const saveTemplate = async () => {
    const normalized = normalizeTemplate(draft);
    if (!normalized.id || !normalized.name) {
      setToast({ message: 'Template id and name are required', type: 'error' });
      return;
    }

    try {
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
      setToast({ message: 'Prompt template saved', type: 'success' });
    } catch (e) {
      setToast({ message: `Save failed: ${e}`, type: 'error' });
    }
  };

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

  const makeDefault = () => {
    if (!draft.id.trim()) return;
    const id = draft.id.trim();
    localStorage.setItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE, id);
    setDefaultTemplateId(id);
    setToast({ message: 'Default prompt template updated', type: 'success' });
  };

  return (
    <div className="settings-section settings-section--fill">
      <div className="settings-header">
        <h3 className="section-title section-title--flush">Prompt Templates</h3>
        <Button variant="outline" size="sm" onClick={createTemplate}>
          <Plus size={13} />
          <span>New Template</span>
        </Button>
      </div>

      {loading ? (
        <div className="section-loading">Loading...</div>
      ) : (
        <div className="sub-list-layout">
          <div className="sub-list-sidebar">
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
                </button>
              ))}
              {templates.length === 0 && (
                <div className="settings-empty">No prompt templates</div>
              )}
            </div>
          </div>

          <div className="sub-list-detail prompt-template-detail">
            <SettingsGroup title="Template" bodyVariant="plain">
              <SettingsItem title="Name">
                <Input
                  value={draft.name}
                  onChange={(e) => setDraft((prev) => ({
                    ...prev,
                    name: e.target.value,
                    id: isNew ? slugify(e.target.value) : prev.id,
                  }))}
                />
              </SettingsItem>
              <SettingsItem title="ID">
                <Input
                  value={draft.id}
                  onChange={(e) => setDraft((prev) => ({ ...prev, id: slugify(e.target.value) }))}
                  disabled={!isNew}
                />
              </SettingsItem>
              <SettingsItem title="Description" wide>
                <Textarea
                  value={draft.description ?? ''}
                  rows={2}
                  onChange={(e) => setDraft((prev) => ({ ...prev, description: e.target.value }))}
                />
              </SettingsItem>
            </SettingsGroup>

            <PromptComposer
              systemPrompt={draft.system_prompt}
              selectedSectionIds={draft.prompt_section_ids}
              promptSections={promptSections}
              mode="general"
              onSystemPromptChange={(value) => setDraft((prev) => ({ ...prev, system_prompt: value }))}
              onSectionToggle={(id) => setDraft((prev) => ({
                ...prev,
                prompt_section_ids: toggleItem(prev.prompt_section_ids, id),
              }))}
            />

            <div className="prompt-template-actions">
              <Button variant="outline" onClick={makeDefault} disabled={!draft.id.trim()}>
                Set Default
              </Button>
              <Button variant="primary" onClick={saveTemplate}>
                Save Template
              </Button>
              <Button variant="danger" onClick={deleteTemplate} disabled={!activeTemplate}>
                <Trash2 size={13} />
                <span>Delete</span>
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function normalizeTemplate(template: UserPromptTemplate): UserPromptTemplate {
  return {
    id: slugify(template.id),
    name: template.name.trim(),
    description: template.description?.trim() || null,
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

function toggleItem(items: string[], item: string): string[] {
  return items.includes(item)
    ? items.filter((candidate) => candidate !== item)
    : [...items, item];
}
