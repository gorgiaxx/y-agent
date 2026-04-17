import type { AgentDetail } from '../../hooks/useAgents';
import type { AgentDraft } from './types';

export function splitList(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

export function formatList(values: string[]): string {
  return values.join(', ');
}

export function toggleItem(values: string[], value: string): string[] {
  return values.includes(value)
    ? values.filter((item) => item !== value)
    : [...values, value];
}

export function slugifyAgentId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

export function buildDraft(detail?: AgentDetail | null): AgentDraft {
  return {
    id: detail?.id ?? '',
    name: detail?.name ?? '',
    description: detail?.description ?? '',
    mode: detail?.mode ?? 'general',
    working_directory: detail?.working_directory ?? '',
    toolcall_enabled: detail?.features.toolcall ?? true,
    skills_enabled: detail?.features.skills ?? true,
    knowledge_enabled: detail?.features.knowledge ?? false,
    allowed_tools: detail?.allowed_tools ?? [],
    system_prompt: detail?.system_prompt ?? '',
    skills: detail?.skills ?? [],
    knowledge_collections: detail?.knowledge_collections ?? [],
    prompt_section_ids: detail?.prompt_section_ids ?? [],
    provider_id: detail?.provider_id ?? '',
    preferred_models: formatList(detail?.preferred_models ?? []),
    fallback_models: formatList(detail?.fallback_models ?? []),
    provider_tags: formatList(detail?.provider_tags ?? []),
    temperature: detail?.temperature?.toString() ?? '',
    top_p: detail?.top_p?.toString() ?? '',
    plan_mode: detail?.plan_mode ?? '',
    thinking_effort: detail?.thinking_effort ?? '',
    permission_mode: detail?.permission_mode ?? '',
    max_iterations: String(detail?.max_iterations ?? 20),
    max_tool_calls: String(detail?.max_tool_calls ?? 50),
    timeout_secs: String(detail?.timeout_secs ?? 300),
    context_sharing: detail?.context_sharing ?? 'none',
    max_context_tokens: String(detail?.max_context_tokens ?? 4096),
    max_completion_tokens: detail?.max_completion_tokens?.toString() ?? '',
    user_callable: detail?.user_callable ?? true,
    mcp_mode: detail?.mcp_mode ?? '',
    mcp_servers: detail?.mcp_servers ?? [],
  };
}

export function serializeAgentDraft(draft: AgentDraft): string {
  const id = draft.id.trim() || slugifyAgentId(draft.name);
  const lines = [
    `id = ${JSON.stringify(id)}`,
    `name = ${JSON.stringify(draft.name.trim())}`,
    `description = ${JSON.stringify(draft.description.trim())}`,
    `mode = ${JSON.stringify(draft.mode)}`,
    'trust_tier = "user_defined"',
    `system_prompt = ${JSON.stringify(draft.system_prompt)}`,
    `toolcall_enabled = ${draft.toolcall_enabled}`,
    `skills_enabled = ${draft.skills_enabled}`,
    `knowledge_enabled = ${draft.knowledge_enabled}`,
    `allowed_tools = [${draft.allowed_tools.map((item) => JSON.stringify(item)).join(', ')}]`,
    `skills = [${draft.skills.map((item) => JSON.stringify(item)).join(', ')}]`,
    `knowledge_collections = [${draft.knowledge_collections.map((item) => JSON.stringify(item)).join(', ')}]`,
    `prompt_section_ids = [${draft.prompt_section_ids.map((item) => JSON.stringify(item)).join(', ')}]`,
    `preferred_models = [${splitList(draft.preferred_models).map((item) => JSON.stringify(item)).join(', ')}]`,
    `fallback_models = [${splitList(draft.fallback_models).map((item) => JSON.stringify(item)).join(', ')}]`,
    `provider_tags = [${splitList(draft.provider_tags).map((item) => JSON.stringify(item)).join(', ')}]`,
    `max_iterations = ${Number.parseInt(draft.max_iterations, 10) || 20}`,
    `max_tool_calls = ${Number.parseInt(draft.max_tool_calls, 10) || 50}`,
    `timeout_secs = ${Number.parseInt(draft.timeout_secs, 10) || 300}`,
    `context_sharing = ${JSON.stringify(draft.context_sharing)}`,
    `max_context_tokens = ${Number.parseInt(draft.max_context_tokens, 10) || 4096}`,
    `user_callable = ${draft.user_callable}`,
  ];

  if (draft.working_directory.trim()) lines.push(`working_directory = ${JSON.stringify(draft.working_directory.trim())}`);
  if (draft.provider_id.trim()) lines.push(`provider_id = ${JSON.stringify(draft.provider_id.trim())}`);
  if (draft.temperature.trim()) lines.push(`temperature = ${Number.parseFloat(draft.temperature)}`);
  if (draft.top_p.trim()) lines.push(`top_p = ${Number.parseFloat(draft.top_p)}`);
  if (draft.plan_mode.trim()) lines.push(`plan_mode = ${JSON.stringify(draft.plan_mode)}`);
  if (draft.thinking_effort.trim()) lines.push(`thinking_effort = ${JSON.stringify(draft.thinking_effort)}`);
  if (draft.permission_mode.trim()) lines.push(`permission_mode = ${JSON.stringify(draft.permission_mode)}`);
  if (draft.max_completion_tokens.trim()) {
    lines.push(`max_completion_tokens = ${Number.parseInt(draft.max_completion_tokens, 10)}`);
  }
  if (draft.mcp_mode.trim()) lines.push(`mcp_mode = ${JSON.stringify(draft.mcp_mode.trim())}`);
  if (draft.mcp_servers.length > 0) {
    lines.push(`mcp_servers = [${draft.mcp_servers.map((item) => JSON.stringify(item)).join(', ')}]`);
  }

  return `${lines.join('\n')}\n`;
}
