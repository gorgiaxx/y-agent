import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface AgentInfo {
  id: string;
  name: string;
  description: string;
  mode: string;
  trust_tier: string;
  capabilities: string[];
  is_overridden: boolean;
}

export interface AgentDetail {
  id: string;
  name: string;
  description: string;
  mode: string;
  trust_tier: string;
  capabilities: string[];
  allowed_tools: string[];
  denied_tools: string[];
  system_prompt: string;
  skills: string[];
  preferred_models: string[];
  fallback_models: string[];
  provider_tags: string[];
  temperature: number | null;
  top_p: number | null;
  max_iterations: number;
  max_tool_calls: number;
  timeout_secs: number;
  context_sharing: string;
  max_context_tokens: number;
  is_overridden: boolean;
}

export function useAgents() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const refreshAgents = useCallback(async () => {
    try {
      const list = await invoke<AgentInfo[]>('agent_list');
      setAgents(list);
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

  return { agents, loading, refreshAgents, getAgentDetail, saveAgent, resetAgent, reloadAgents };
}
