import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface AgentFeatureFlags {
  toolcall: boolean;
  skills: boolean;
  knowledge: boolean;
}

export interface AgentInfo {
  id: string;
  name: string;
  icon?: string | null;
  description: string;
  mode: string;
  trust_tier: string;
  capabilities: string[];
  working_directory?: string | null;
  provider_id?: string | null;
  features: AgentFeatureFlags;
  user_callable: boolean;
  is_overridden: boolean;
}

export interface AgentDetail {
  id: string;
  name: string;
  icon?: string | null;
  description: string;
  mode: string;
  trust_tier: string;
  capabilities: string[];
  working_directory?: string | null;
  allowed_tools: string[];
  system_prompt: string;
  skills: string[];
  features: AgentFeatureFlags;
  knowledge_collections: string[];
  prompt_section_ids: string[];
  provider_id?: string | null;
  preferred_models: string[];
  fallback_models: string[];
  provider_tags: string[];
  temperature: number | null;
  top_p: number | null;
  plan_mode?: string | null;
  thinking_effort?: string | null;
  permission_mode?: string | null;
  max_iterations: number;
  max_tool_calls: number;
  timeout_secs: number;
  context_sharing: string;
  max_context_tokens: number;
  max_completion_tokens: number | null;
  user_callable: boolean;
  is_overridden: boolean;
}

export interface AgentToolInfo {
  name: string;
  description: string;
  category: string;
  is_dangerous: boolean;
}

export interface PromptSectionInfo {
  id: string;
  category: string;
}

export interface AgentSourceInfo {
  path: string;
  content: string;
  is_user_file: boolean;
}

export function useAgents() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [tools, setTools] = useState<AgentToolInfo[]>([]);
  const [promptSections, setPromptSections] = useState<PromptSectionInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const refreshAgents = useCallback(async () => {
    try {
      const [list, toolList, sectionList] = await Promise.all([
        invoke<AgentInfo[]>('agent_list'),
        invoke<AgentToolInfo[]>('agent_tool_list'),
        invoke<PromptSectionInfo[]>('agent_prompt_section_list'),
      ]);
      setAgents(list);
      setTools(toolList);
      setPromptSections(sectionList);
    } catch (e) {
      console.error('Failed to list agents:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshAgents();
  }, [refreshAgents]);

  const getAgentDetail = useCallback(async (id: string): Promise<AgentDetail | null> => {
    try {
      return await invoke<AgentDetail>('agent_get', { id });
    } catch (e) {
      console.error('Failed to get agent detail:', e);
      return null;
    }
  }, []);

  const getAgentSource = useCallback(async (id: string): Promise<AgentSourceInfo | null> => {
    try {
      return await invoke<AgentSourceInfo>('agent_source_get', { id });
    } catch (e) {
      console.error('Failed to get agent source:', e);
      return null;
    }
  }, []);

  const parseAgentToml = useCallback(async (tomlContent: string): Promise<AgentDetail | null> => {
    try {
      return await invoke<AgentDetail>('agent_toml_parse', { tomlContent });
    } catch (e) {
      console.error('Failed to parse agent TOML:', e);
      return null;
    }
  }, []);

  const saveAgent = useCallback(async (id: string, tomlContent: string): Promise<boolean> => {
    try {
      await invoke('agent_save', { id, tomlContent });
      await refreshAgents();
      return true;
    } catch (e) {
      console.error('Failed to save agent:', e);
      return false;
    }
  }, [refreshAgents]);

  const resetAgent = useCallback(async (id: string): Promise<boolean> => {
    try {
      await invoke('agent_reset', { id });
      await refreshAgents();
      return true;
    } catch (e) {
      console.error('Failed to reset agent:', e);
      return false;
    }
  }, [refreshAgents]);

  const reloadAgents = useCallback(async (): Promise<boolean> => {
    try {
      await invoke('agent_reload');
      await refreshAgents();
      return true;
    } catch (e) {
      console.error('Failed to reload agents:', e);
      return false;
    }
  }, [refreshAgents]);

  return {
    agents,
    tools,
    promptSections,
    loading,
    refreshAgents,
    getAgentDetail,
    getAgentSource,
    parseAgentToml,
    saveAgent,
    resetAgent,
    reloadAgents,
  };
}
