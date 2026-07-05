// ---------------------------------------------------------------------------
// usePromptTemplates -- load prompt templates and apply/clear session prompt config.
//
// Shared by InputArea (quick-apply dropdown), SessionPromptPanel (full editor),
// and PromptTemplatesTab (CRUD). Centralizes the three transport.invoke calls
// that were duplicated across those components.
// ---------------------------------------------------------------------------

import { useState, useCallback } from 'react';
import { transport, logger } from '../lib';
import type { SessionPromptConfig, UserPromptTemplate } from '../types';

export interface UsePromptTemplatesReturn {
  templates: UserPromptTemplate[];
  loading: boolean;
  /** ID of the template currently being applied, or '__clear__' for clear. */
  applying: string | null;
  loadTemplates: () => Promise<void>;
  applyTemplate: (sessionId: string, template: UserPromptTemplate) => Promise<boolean>;
  clearSessionPrompt: (sessionId: string) => Promise<boolean>;
}

export function usePromptTemplates(onApplied?: () => void): UsePromptTemplatesReturn {
  const [templates, setTemplates] = useState<UserPromptTemplate[]>([]);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState<string | null>(null);

  const loadTemplates = useCallback(async () => {
    setLoading(true);
    try {
      const list = await transport.invoke<UserPromptTemplate[]>('prompt_template_list');
      setTemplates(list);
    } catch (e) {
      logger.error('[usePromptTemplates] load error:', e);
      setTemplates([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const applyTemplate = useCallback(async (sessionId: string, template: UserPromptTemplate) => {
    setApplying(template.id);
    try {
      const config: SessionPromptConfig = {
        system_prompt: template.system_prompt,
        prompt_section_ids: template.prompt_section_ids,
        template_id: template.id,
      };
      await transport.invoke('session_set_prompt_config', { sessionId, config });
      onApplied?.();
      return true;
    } catch (e) {
      logger.error('[usePromptTemplates] apply error:', e);
      return false;
    } finally {
      setApplying(null);
    }
  }, [onApplied]);

  const clearSessionPrompt = useCallback(async (sessionId: string) => {
    setApplying('__clear__');
    try {
      const config: SessionPromptConfig = {
        system_prompt: null,
        prompt_section_ids: [],
        template_id: null,
      };
      await transport.invoke('session_set_prompt_config', { sessionId, config });
      onApplied?.();
      return true;
    } catch (e) {
      logger.error('[usePromptTemplates] clear error:', e);
      return false;
    } finally {
      setApplying(null);
    }
  }, [onApplied]);

  return { templates, loading, applying, loadTemplates, applyTemplate, clearSessionPrompt };
}
