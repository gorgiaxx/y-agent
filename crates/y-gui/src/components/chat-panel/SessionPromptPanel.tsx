import { useState, useEffect, useCallback } from 'react';

import { transport } from '../../lib';
import type { SessionPromptConfig, UserPromptTemplate } from '../../types';
import type { PromptSectionInfo } from '../../hooks/useAgents';
import {
  Button,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  SettingsGroup,
  SettingsItem,
} from '../ui';
import { PromptComposer } from '../prompts/PromptComposer';

import './SessionPromptPanel.css';

interface SessionPromptPanelProps {
  sessionId: string;
  sessionTitle?: string | null;
  onSaved: (hasPrompt: boolean) => void;
}

export function SessionPromptPanel({
  sessionId,
  sessionTitle,
  onSaved,
}: SessionPromptPanelProps) {
  const [promptConfig, setPromptConfig] = useState<SessionPromptConfig>({
    system_prompt: '',
    prompt_section_ids: [],
    template_id: null,
  });
  const [promptSections, setPromptSections] = useState<PromptSectionInfo[]>([]);
  const [templates, setTemplates] = useState<UserPromptTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ message: string; type: 'success' | 'error' } | null>(null);

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
    } catch (e) {
      setPromptConfig({ system_prompt: '', prompt_section_ids: [], template_id: null });
      setPromptSections([]);
      setTemplates([]);
      setToast({ message: `Failed to load session prompt: ${e}`, type: 'error' });
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    loadPrompt();
  }, [loadPrompt]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(timer);
  }, [toast]);

  const handleSave = async () => {
    setSaving(true);
    try {
      const config = normalizePromptConfig(promptConfig);
      await transport.invoke('session_set_prompt_config', {
        sessionId,
        config,
      });
      onSaved(hasPromptConfig(config));
      setPromptConfig({
        system_prompt: config.system_prompt ?? '',
        prompt_section_ids: config.prompt_section_ids,
        template_id: config.template_id,
      });
      setToast({ message: 'Session prompt saved', type: 'success' });
    } catch (e) {
      setToast({ message: `Save failed: ${e}`, type: 'error' });
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
      setToast({ message: 'Session prompt cleared', type: 'success' });
    } catch (e) {
      setToast({ message: `Clear failed: ${e}`, type: 'error' });
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
  };

  return (
    <div className="settings-panel session-prompt-panel">
      <div className="settings-action-bar" data-tauri-drag-region>
        <h2 className="settings-action-bar-title">
          {sessionTitle ? `Session Prompt: ${sessionTitle}` : 'Session Prompt'}
        </h2>
        <div className="settings-action-bar-actions">
          <Button
            type="button"
            variant="ghost"
            onClick={handleClear}
            disabled={loading || saving || !hasPromptConfig(promptConfig)}
          >
            Clear
          </Button>
          <Button type="button" variant="primary" onClick={handleSave} disabled={loading || saving}>
            {saving ? 'Saving...' : 'Save Changes'}
          </Button>
        </div>
      </div>

      <ScrollArea className="flex-1 min-h-0">
        <div className="settings-content">
          <SettingsGroup title="Template">
            <SettingsItem title="Prompt Template" wide>
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
            </SettingsItem>
          </SettingsGroup>

          {loading ? (
            <div className="section-loading">Loading...</div>
          ) : (
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
              onSelectedSectionIdsChange={(ids) => setPromptConfig((prev) => ({
                ...prev,
                template_id: null,
                prompt_section_ids: ids,
              }))}
            />
          )}
        </div>
      </ScrollArea>

      {toast && (
        <div className={`settings-toast ${toast.type}`}>
          {toast.message}
        </div>
      )}

    </div>
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
