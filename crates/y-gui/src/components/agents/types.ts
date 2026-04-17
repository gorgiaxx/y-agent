export type EditorTab = 'general' | 'tools' | 'skills' | 'knowledge' | 'prompt' | 'model' | 'limits';
export type EditorSurface = 'form' | 'raw';

export interface AgentDraft {
  id: string;
  name: string;
  description: string;
  mode: string;
  working_directory: string;
  toolcall_enabled: boolean;
  skills_enabled: boolean;
  knowledge_enabled: boolean;
  allowed_tools: string[];
  system_prompt: string;
  skills: string[];
  knowledge_collections: string[];
  prompt_section_ids: string[];
  provider_id: string;
  preferred_models: string;
  fallback_models: string;
  provider_tags: string;
  temperature: string;
  top_p: string;
  plan_mode: string;
  thinking_effort: string;
  permission_mode: string;
  max_iterations: string;
  max_tool_calls: string;
  timeout_secs: string;
  context_sharing: string;
  max_context_tokens: string;
  max_completion_tokens: string;
  user_callable: boolean;
  mcp_mode: string;
  mcp_servers: string[];
}

export const EDITOR_TABS: { id: EditorTab; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'tools', label: 'Tools' },
  { id: 'skills', label: 'Skills' },
  { id: 'knowledge', label: 'Knowledge' },
  { id: 'prompt', label: 'Prompt' },
  { id: 'model', label: 'Model' },
  { id: 'limits', label: 'Limits' },
];
