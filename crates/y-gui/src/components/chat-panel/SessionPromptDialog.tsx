import { useState, useEffect, useCallback } from 'react';
import { transport } from '../../lib';
import { STORAGE_KEYS } from '../../constants/storageKeys';
import type { SessionPromptConfig, UserPromptTemplate } from '../../types';
import type { PromptSectionInfo } from '../../hooks/useAgents';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui';
import { PromptComposer } from '../prompts/PromptComposer';
import './SessionPromptDialog.css';

interface SessionPromptDialogProps {
  sessionId: string;
  onClose: () => void;
  onSaved: (hasPrompt: boolean) => void;
}

export function SessionPromptDialog({
  sessionId,
  onClose,
  onSaved,
}: SessionPromptDialogProps) {
  const [promptConfig, setPromptConfig] = useState<SessionPromptConfig>({
    system_prompt: '',
    prompt_section_ids: [],
    template_id: null,
  });
  const [promptSections, setPromptSections] = useState<PromptSectionInfo[]>([]);
  const [templates, setTemplates] = useState<UserPromptTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const loadPrompt = useCallback(async () => {
    setLoading(true);
    try {
      const [current, sections, templateList] = await Promise.all([
        transport.invoke<SessionPromptConfig>('session_get_prompt_config', { sessionId }),
        transport.invoke<PromptSectionInfo[]>('agent_prompt_section_list'),
        transport.invoke<UserPromptTemplate[]>('prompt_template_list'),
      ]);
      setPromptConfig({
        system_prompt: current.system_prompt ?? '',
        prompt_section_ids: current.prompt_section_ids ?? [],
        template_id: current.template_id ?? null,
      });
      setPromptSections(sections);
      setTemplates(templateList);
    } catch {
      setPromptConfig({ system_prompt: '', prompt_section_ids: [], template_id: null });
      setPromptSections([]);
      setTemplates([]);
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    loadPrompt();
  }, [loadPrompt]);

  const handleSave = async () => {
    setSaving(true);
    try {
      const config = normalizePromptConfig(promptConfig);
      await transport.invoke('session_set_prompt_config', {
        sessionId,
        config,
      });
      if (config.template_id) {
        localStorage.setItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE, config.template_id);
      }
      onSaved(hasPromptConfig(config));
    } catch (e) {
      console.error('Failed to save session prompt:', e);
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async () => {
    setSaving(true);
    try {
      await transport.invoke('session_set_prompt_config', {
        sessionId,
        config: { system_prompt: null, prompt_section_ids: [], template_id: null },
      });
      setPromptConfig({ system_prompt: '', prompt_section_ids: [], template_id: null });
      onSaved(false);
    } catch (e) {
      console.error('Failed to clear session prompt:', e);
    } finally {
      setSaving(false);
    }
  };

  const applyTemplate = (templateId: string) => {
    if (templateId === '__none__') {
      setPromptConfig((prev) => ({ ...prev, template_id: null }));
      return;
    }
    const template = templates.find((item) => item.id === templateId);
    if (!template) {
      return;
    }
    setPromptConfig({
      system_prompt: template.system_prompt,
      prompt_section_ids: template.prompt_section_ids,
      template_id: template.id,
    });
    localStorage.setItem(STORAGE_KEYS.DEFAULT_PROMPT_TEMPLATE, template.id);
  };

  const toggleSection = (id: string) => {
    setPromptConfig((prev) => ({
      ...prev,
      template_id: null,
      prompt_section_ids: toggleItem(prev.prompt_section_ids, id),
    }));
  };

  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent size="lg" className="text-left items-stretch">
        <DialogTitle className="text-left">
          Session System Prompt
        </DialogTitle>

        <div className="flex flex-col gap-2 mt-2">
          <Select
            value={promptConfig.template_id ?? '__none__'}
            onValueChange={applyTemplate}
          >
            <SelectTrigger>
              <SelectValue placeholder="Template" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">No Template</SelectItem>
              {templates.map((template) => (
                <SelectItem key={template.id} value={template.id}>
                  {template.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          {loading ? (
            <div className="text-12px text-[var(--text-muted)] py-4 text-center">
              Loading...
            </div>
          ) : (
            <div className="session-prompt-composer">
              <PromptComposer
                systemPrompt={promptConfig.system_prompt ?? ''}
                selectedSectionIds={promptConfig.prompt_section_ids}
                promptSections={promptSections}
                mode="general"
                onSystemPromptChange={(value) => setPromptConfig((prev) => ({
                  ...prev,
                  system_prompt: value,
                  template_id: null,
                }))}
                onSectionToggle={toggleSection}
              />
            </div>
          )}

          <div className="flex gap-2 justify-between mt-1">
            <Button
              type="button"
              variant="ghost"
              onClick={handleClear}
              disabled={loading || saving || !hasPromptConfig(promptConfig)}
            >
              Clear
            </Button>
            <div className="flex gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="primary"
                onClick={handleSave}
                disabled={loading || saving}
              >
                {saving ? 'Saving...' : 'Save'}
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function normalizePromptConfig(config: SessionPromptConfig): SessionPromptConfig {
  return {
    system_prompt: config.system_prompt?.trim() || null,
    prompt_section_ids: dedupe(config.prompt_section_ids),
    template_id: config.template_id?.trim() || null,
  };
}

function hasPromptConfig(config: SessionPromptConfig): boolean {
  return Boolean(
    config.system_prompt?.trim()
    || config.prompt_section_ids.length > 0
    || config.template_id?.trim(),
  );
}

function dedupe(values: string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

function toggleItem(items: string[], item: string): string[] {
  return items.includes(item)
    ? items.filter((candidate) => candidate !== item)
    : [...items, item];
}
