// Command-to-endpoint mapping for HttpTransport.
//
// Maps every Tauri invoke command name to its corresponding y-web REST endpoint.
// The `transform` function (optional) reshapes Tauri-style args into the format
// expected by the HTTP endpoint (path params, query params, body).

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH';

export interface EndpointDef {
  method: HttpMethod;
  path: string | ((args: Record<string, unknown>) => string);
  query?: (args: Record<string, unknown>) => Record<string, string | undefined>;
  body?: (args: Record<string, unknown>) => unknown;
  response?: (value: unknown) => unknown;
}

function id(args: Record<string, unknown>) {
  return args;
}

function arg(args: Record<string, unknown>, ...keys: string[]) {
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(args, key)) {
      return args[key];
    }
  }
  return undefined;
}

function compactBody(values: Record<string, unknown>) {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(values)) {
    if (value !== undefined) {
      out[key] = value;
    }
  }
  return out;
}

function pathWith(template: string, ...keys: string[]) {
  return (args: Record<string, unknown>) => {
    let p = template;
    for (const k of keys) {
      p = p.replace(`{${k}}`, encodeURIComponent(String(args[k] ?? '')));
    }
    return p;
  };
}

function pathWithArg(template: string, placeholder: string, ...keys: string[]) {
  return (args: Record<string, unknown>) => (
    template.replace(`{${placeholder}}`, encodeURIComponent(String(arg(args, ...keys) ?? '')))
  );
}

function backgroundTaskPath(action?: 'poll' | 'write' | 'kill') {
  return (args: Record<string, unknown>) => {
    const sessionId = encodeURIComponent(String(arg(args, 'sessionId', 'session_id') ?? ''));
    if (!action) return `/api/v1/sessions/${sessionId}/background-tasks`;

    const processId = encodeURIComponent(String(arg(args, 'processId', 'process_id') ?? ''));
    return `/api/v1/sessions/${sessionId}/background-tasks/${processId}/${action}`;
  };
}

function bodyWithout(...exclude: string[]) {
  const set = new Set(exclude);
  return (args: Record<string, unknown>) => {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(args)) {
      if (!set.has(k)) out[k] = v;
    }
    return out;
  };
}

function contentResponse(value: unknown) {
  if (value && typeof value === 'object' && 'content' in value) {
    return (value as { content: unknown }).content;
  }
  return value;
}

function messageResponse(value: unknown) {
  if (value && typeof value === 'object' && 'message' in value) {
    return (value as { message: unknown }).message;
  }
  return value;
}

function resultResponse(value: unknown) {
  if (value && typeof value === 'object' && 'result' in value) {
    return (value as { result: unknown }).result;
  }
  return value;
}

function textResponse(value: unknown) {
  if (value && typeof value === 'object' && 'text' in value) {
    return (value as { text: unknown }).text;
  }
  return value;
}

function propertyResponse(key: string) {
  return (value: unknown) => {
    if (value && typeof value === 'object' && key in value) {
      return (value as Record<string, unknown>)[key];
    }
    return value;
  };
}

function successResponse(value: unknown) {
  if (value && typeof value === 'object' && 'message' in value) {
    return true;
  }
  return value;
}

function systemStatusResponse(value: unknown) {
  if (!value || typeof value !== 'object' || !('status' in value)) {
    return value;
  }
  const status = (value as { status?: Record<string, unknown> }).status;
  if (!status) {
    return value;
  }
  return {
    version: status.version ?? 'unknown',
    healthy: true,
    provider_count: status.providers_registered ?? 0,
    session_count: null,
  };
}

function encodePathSegments(path: unknown) {
  return String(path ?? '')
    .split('/')
    .map((segment) => encodeURIComponent(segment))
    .join('/');
}

function unwrapRequestBody(args: Record<string, unknown>) {
  const request = arg(args, 'request');
  if (request && typeof request === 'object') {
    return request;
  }
  return bodyWithout('id')(args);
}

function contentBody(args: Record<string, unknown>) {
  return arg(args, 'content') ?? args;
}

function sessionCreateBody(a: Record<string, unknown>) {
  return compactBody({
    title: arg(a, 'title'),
    agent_id: arg(a, 'agentId', 'agent_id'),
  });
}

function chatSendBody(a: Record<string, unknown>) {
  return {
    message: arg(a, 'message'),
    session_id: arg(a, 'sessionId', 'session_id'),
    provider_id: arg(a, 'providerId', 'provider_id'),
    request_mode: arg(a, 'requestMode', 'request_mode'),
    skills: arg(a, 'skills'),
    knowledge_collections: arg(a, 'knowledgeCollections', 'knowledge_collections'),
    context_start_index: arg(a, 'contextStartIndex', 'context_start_index'),
    thinking_effort: arg(a, 'thinkingEffort', 'thinking_effort'),
    attachments: arg(a, 'attachments'),
    plan_mode: arg(a, 'planMode', 'plan_mode'),
    mcp_mode: arg(a, 'mcpMode', 'mcp_mode'),
    mcp_servers: arg(a, 'mcpServers', 'mcp_servers'),
    image_generation_options: arg(a, 'imageGenerationOptions', 'image_generation_options'),
  };
}

function chatResendBody(a: Record<string, unknown>) {
  return {
    session_id: arg(a, 'sessionId', 'session_id'),
    checkpoint_id: arg(a, 'checkpointId', 'checkpoint_id'),
    provider_id: arg(a, 'providerId', 'provider_id'),
    request_mode: arg(a, 'requestMode', 'request_mode'),
    knowledge_collections: arg(a, 'knowledgeCollections', 'knowledge_collections'),
    thinking_effort: arg(a, 'thinkingEffort', 'thinking_effort'),
    plan_mode: arg(a, 'planMode', 'plan_mode'),
  };
}

function agentTomlBody(a: Record<string, unknown>) {
  return {
    toml_content: arg(a, 'tomlContent', 'toml_content'),
  };
}

function providerTestBody(a: Record<string, unknown>) {
  return compactBody({
    provider_type: arg(a, 'providerType', 'provider_type'),
    model: arg(a, 'model'),
    api_key: arg(a, 'apiKey', 'api_key'),
    api_key_env: arg(a, 'apiKeyEnv', 'api_key_env'),
    base_url: arg(a, 'baseUrl', 'base_url'),
    headers: arg(a, 'headers'),
    http_protocol: arg(a, 'httpProtocol', 'http_protocol'),
    tags: arg(a, 'tags'),
    capabilities: arg(a, 'capabilities'),
    probe_mode: arg(a, 'probeMode', 'probe_mode'),
  });
}

function providerListModelsBody(a: Record<string, unknown>) {
  return compactBody({
    base_url: arg(a, 'baseUrl', 'base_url'),
    api_key: arg(a, 'apiKey', 'api_key'),
    api_key_env: arg(a, 'apiKeyEnv', 'api_key_env'),
    headers: arg(a, 'headers'),
    http_protocol: arg(a, 'httpProtocol', 'http_protocol'),
  });
}

function knowledgeIngestBody(a: Record<string, unknown>) {
  return compactBody({
    source: arg(a, 'source'),
    domain: arg(a, 'domain'),
    collection: arg(a, 'collection'),
    use_llm_summary: arg(a, 'useLlmSummary', 'use_llm_summary'),
    extract_metadata: arg(a, 'extractMetadata', 'extract_metadata'),
  });
}

function knowledgeBatchIngestBody(a: Record<string, unknown>) {
  return compactBody({
    sources: arg(a, 'sources'),
    domain: arg(a, 'domain'),
    collection: arg(a, 'collection'),
    use_llm_summary: arg(a, 'useLlmSummary', 'use_llm_summary'),
    extract_metadata: arg(a, 'extractMetadata', 'extract_metadata'),
  });
}

function knowledgeMetadataBody(a: Record<string, unknown>) {
  return compactBody({
    document_type: arg(a, 'documentType', 'document_type'),
    industry: arg(a, 'industry'),
    subcategory: arg(a, 'subcategory'),
    interpreted_title: arg(a, 'interpretedTitle', 'interpreted_title'),
    tags: arg(a, 'tags'),
  });
}

function rewindTargetBody(a: Record<string, unknown>) {
  return {
    target_message_id: arg(a, 'targetMessageId', 'target_message_id'),
  };
}

// prettier-ignore
export const COMMAND_MAP: Record<string, EndpointDef> = {
  // -- System / Health --
  health_check:        { method: 'GET',  path: '/health' },
  system_status:       { method: 'GET',  path: '/api/v1/status', response: systemStatusResponse },
  provider_list:       { method: 'GET',  path: '/api/v1/providers' },
  app_paths:           { method: 'GET',  path: '/api/v1/app-paths' },

  // -- Sessions --
  session_list:        { method: 'GET',  path: '/api/v1/sessions',
                         query: (a) => a.agentId ? { agent_id: String(a.agentId) } : {} },
  session_create:      { method: 'POST', path: '/api/v1/sessions', body: sessionCreateBody },
  session_get_messages: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/messages', 'sessionId'),
                         query: (a) => a.last ? { last: String(a.last) } : {} },
  session_delete:      { method: 'DELETE', path: pathWith('/api/v1/sessions/{sessionId}', 'sessionId') },
  session_truncate_messages: { method: 'POST', path: pathWith('/api/v1/sessions/{sessionId}/truncate', 'sessionId'),
                         body: (a) => ({ keep_count: a.keepCount }) },
  session_get_context_reset: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/context-reset', 'sessionId'), response: propertyResponse('index') },
  session_set_context_reset: { method: 'PUT', path: pathWith('/api/v1/sessions/{sessionId}/context-reset', 'sessionId'),
                         body: (a) => ({ index: a.index }) },
  session_get_custom_prompt: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/custom-prompt', 'sessionId'), response: propertyResponse('prompt') },
  session_set_custom_prompt: { method: 'PUT', path: pathWith('/api/v1/sessions/{sessionId}/custom-prompt', 'sessionId'),
                         body: (a) => ({ prompt: a.prompt }) },
  session_fork:        { method: 'POST', path: pathWith('/api/v1/sessions/{sessionId}/fork', 'sessionId'),
                         body: (a) => ({ message_index: arg(a, 'messageIndex', 'message_index', 'atIndex'), title: arg(a, 'title') }) },
  session_rename:      { method: 'PUT',  path: pathWith('/api/v1/sessions/{sessionId}/rename', 'sessionId'),
                         body: (a) => ({ title: a.title }) },

  // -- Chat --
  chat_send:           { method: 'POST', path: '/api/v1/chat/send', body: chatSendBody },
  chat_cancel:         { method: 'POST', path: '/api/v1/chat/cancel', body: (a) => ({ run_id: a.runId }) },
  chat_undo:           { method: 'POST', path: '/api/v1/chat/undo',
                         body: (a) => ({ session_id: arg(a, 'sessionId', 'session_id'), checkpoint_id: arg(a, 'checkpointId', 'checkpoint_id') }) },
  chat_resend:         { method: 'POST', path: '/api/v1/chat/resend', body: chatResendBody },
  chat_checkpoint_list: { method: 'GET', path: pathWith('/api/v1/chat/checkpoints/{sessionId}', 'sessionId') },
  chat_find_checkpoint_for_resend: { method: 'POST', path: '/api/v1/chat/find-checkpoint',
                         body: (a) => ({ session_id: arg(a, 'sessionId', 'session_id'), user_message_content: arg(a, 'userMessageContent', 'user_message_content'), message_id: arg(a, 'messageId', 'message_id') }) },
  chat_get_messages_with_status: { method: 'GET', path: pathWith('/api/v1/chat/messages-with-status/{sessionId}', 'sessionId') },
  chat_restore_branch: { method: 'POST', path: '/api/v1/chat/restore-branch',
                         body: (a) => ({ session_id: arg(a, 'sessionId', 'session_id'), checkpoint_id: arg(a, 'checkpointId', 'checkpoint_id') }) },
  context_compact:     { method: 'POST', path: pathWith('/api/v1/chat/compact/{sessionId}', 'sessionId') },
  chat_answer_question: { method: 'POST', path: '/api/v1/chat/answer-question',
                         body: (a) => ({ interaction_id: arg(a, 'interactionId', 'interaction_id'), answers: arg(a, 'answers') }) },
  chat_answer_permission: { method: 'POST', path: '/api/v1/chat/answer-permission',
                         body: (a) => ({ request_id: arg(a, 'requestId', 'request_id'), decision: arg(a, 'decision') }) },
  session_last_turn_meta: { method: 'GET', path: pathWith('/api/v1/chat/last-turn-meta/{sessionId}', 'sessionId') },
  // -- Agents --
  agent_list:          { method: 'GET',  path: '/api/v1/agents' },
  agent_get:           { method: 'GET',  path: pathWith('/api/v1/agents/{id}', 'id') },
  agent_source_get:    { method: 'GET',  path: pathWith('/api/v1/agents/{id}/source', 'id') },
  agent_toml_parse:    { method: 'POST', path: '/api/v1/agents/parse-toml', body: agentTomlBody },
  agent_save:          { method: 'PUT',  path: pathWith('/api/v1/agents/{id}', 'id'),
                         body: agentTomlBody },
  agent_reset:         { method: 'POST', path: pathWith('/api/v1/agents/{id}/reset', 'id') },
  agent_reload:        { method: 'POST', path: '/api/v1/agents/reload' },
  agent_tool_list:     { method: 'GET',  path: '/api/v1/agents/tools' },
  agent_prompt_section_list: { method: 'GET', path: '/api/v1/agents/prompt-sections' },
  translate_text:      { method: 'POST', path: '/api/v1/agents/translate', body: id, response: textResponse },

  // -- Config --
  config_get:          { method: 'GET',  path: '/api/v1/config' },
  config_set_section:  { method: 'PUT',  path: pathWith('/api/v1/config/{section}', 'section'),
                         body: bodyWithout('section') },
  config_get_section:  { method: 'GET',  path: pathWith('/api/v1/config/{section}', 'section'), response: contentResponse },
  config_save_section: { method: 'PUT',  path: pathWith('/api/v1/config/{section}', 'section'),
                         body: (a) => ({ content: a.content }) },
  config_reload:       { method: 'POST', path: '/api/v1/config/reload', response: messageResponse },
  provider_test:       { method: 'POST', path: '/api/v1/providers/test', body: providerTestBody, response: resultResponse },
  provider_list_models: { method: 'POST', path: '/api/v1/providers/list-models', body: providerListModelsBody },
  mcp_config_get:      { method: 'GET',  path: '/api/v1/config/mcp' },
  mcp_config_save:     { method: 'PUT',  path: '/api/v1/config/mcp', body: contentBody },
  prompt_list:         { method: 'GET',  path: '/api/v1/config/prompts' },
  prompt_get:          { method: 'GET',  path: pathWith('/api/v1/config/prompts/{filename}', 'filename'), response: contentResponse },
  prompt_save:         { method: 'PUT',  path: pathWith('/api/v1/config/prompts/{filename}', 'filename'),
                         body: (a) => ({ content: a.content }) },
  prompt_get_default:  { method: 'GET',  path: pathWith('/api/v1/config/prompts/{filename}/default', 'filename'), response: contentResponse },
  // -- Knowledge --
  kb_collection_list:  { method: 'GET',  path: '/api/v1/knowledge/collections' },
  kb_collection_create: { method: 'POST', path: '/api/v1/knowledge/collections', body: id },
  kb_collection_delete: { method: 'DELETE', path: pathWith('/api/v1/knowledge/collections/{name}', 'name') },
  kb_collection_rename: { method: 'POST', path: pathWithArg('/api/v1/knowledge/collections/{name}/rename', 'name', 'name', 'oldName'),
                         body: (a) => ({ new_name: arg(a, 'newName', 'new_name') }) },
  kb_entry_list:       { method: 'GET',  path: pathWith('/api/v1/knowledge/collections/{collection}/entries', 'collection') },
  kb_entry_detail:     { method: 'GET',  path: pathWithArg('/api/v1/knowledge/entries/{id}', 'id', 'id', 'entryId'),
                         query: (a) => a.resolution ? { resolution: String(a.resolution) } : {} },
  kb_entry_delete:     { method: 'DELETE', path: pathWithArg('/api/v1/knowledge/entries/{id}', 'id', 'id', 'entryId') },
  kb_entry_update_metadata: { method: 'PATCH', path: pathWithArg('/api/v1/knowledge/entries/{id}/metadata', 'id', 'id', 'entryId'),
                         body: knowledgeMetadataBody },
  kb_search:           { method: 'POST', path: '/api/v1/knowledge/search', body: id },
  kb_ingest:           { method: 'POST', path: '/api/v1/knowledge/ingest', body: knowledgeIngestBody },
  kb_ingest_batch:     { method: 'POST', path: '/api/v1/knowledge/ingest-batch', body: knowledgeBatchIngestBody },
  kb_expand_folder:    { method: 'POST', path: '/api/v1/knowledge/expand-folder', body: id },
  kb_stats:            { method: 'GET',  path: '/api/v1/knowledge/stats' },

  // -- Skills --
  skill_list:          { method: 'GET',  path: '/api/v1/skills' },
  skill_get:           { method: 'GET',  path: pathWith('/api/v1/skills/{name}', 'name') },
  skill_uninstall:     { method: 'DELETE', path: pathWith('/api/v1/skills/{name}', 'name') },
  skill_set_enabled:   { method: 'PUT',  path: pathWith('/api/v1/skills/{name}/enabled', 'name'),
                         body: (a) => ({ enabled: a.enabled }) },
  skill_get_files:     { method: 'GET',  path: pathWith('/api/v1/skills/{name}/files', 'name') },
  skill_read_file:     { method: 'GET',  path: (a) => `/api/v1/skills/${encodeURIComponent(String(a.name))}/files/${encodePathSegments(arg(a, 'relativePath', 'path'))}`, response: contentResponse },
  skill_save_file:     { method: 'PUT',  path: (a) => `/api/v1/skills/${encodeURIComponent(String(a.name))}/files/${encodePathSegments(arg(a, 'relativePath', 'path'))}`,
                         body: (a) => ({ content: a.content }) },
  skill_import:        { method: 'POST', path: '/api/v1/skills/import', body: id },

  // -- Workflows --
  workflow_list:       { method: 'GET',  path: '/api/v1/workflows' },
  workflow_get:        { method: 'GET',  path: pathWith('/api/v1/workflows/{id}', 'id') },
  workflow_create:     { method: 'POST', path: '/api/v1/workflows', body: id },
  workflow_update:     { method: 'PUT',  path: pathWith('/api/v1/workflows/{id}', 'id'), body: bodyWithout('id') },
  workflow_delete:     { method: 'DELETE', path: pathWith('/api/v1/workflows/{id}', 'id'), response: successResponse },
  workflow_validate:   { method: 'POST', path: '/api/v1/workflows/validate', body: id },
  workflow_dag:        { method: 'GET',  path: pathWith('/api/v1/workflows/{id}/dag', 'id') },
  workflow_execute:    { method: 'POST', path: pathWithArg('/api/v1/workflows/{id}/execute', 'id', 'id', 'workflowId') },
  // -- Schedules --
  schedule_list:       { method: 'GET',  path: '/api/v1/schedules' },
  schedule_get:        { method: 'GET',  path: pathWith('/api/v1/schedules/{id}', 'id') },
  schedule_create:     { method: 'POST', path: '/api/v1/schedules', body: unwrapRequestBody },
  schedule_update:     { method: 'PUT',  path: pathWith('/api/v1/schedules/{id}', 'id'), body: unwrapRequestBody },
  schedule_delete:     { method: 'DELETE', path: pathWith('/api/v1/schedules/{id}', 'id'), response: successResponse },
  schedule_pause:      { method: 'POST', path: pathWith('/api/v1/schedules/{id}/pause', 'id') },
  schedule_resume:     { method: 'POST', path: pathWith('/api/v1/schedules/{id}/resume', 'id') },
  schedule_execution_history: { method: 'GET', path: pathWithArg('/api/v1/schedules/{id}/executions', 'id', 'id', 'scheduleId') },
  schedule_execution_get: { method: 'GET', path: pathWith('/api/v1/schedules/executions/{executionId}', 'executionId') },
  schedule_trigger_now: { method: 'POST', path: pathWithArg('/api/v1/schedules/{id}/trigger', 'id', 'id', 'scheduleId') },

  // -- Workspaces --
  workspace_list:      { method: 'GET',  path: '/api/v1/workspaces' },
  workspace_create:    { method: 'POST', path: '/api/v1/workspaces', body: id },
  workspace_update:    { method: 'PUT',  path: pathWith('/api/v1/workspaces/{id}', 'id'), body: bodyWithout('id') },
  workspace_delete:    { method: 'DELETE', path: pathWith('/api/v1/workspaces/{id}', 'id') },
  workspace_session_map: { method: 'GET', path: '/api/v1/workspaces/session-map' },
  workspace_assign_session: { method: 'POST', path: '/api/v1/workspaces/assign',
                         body: (a) => ({ workspace_id: arg(a, 'workspaceId', 'workspace_id'), session_id: arg(a, 'sessionId', 'session_id') }) },
  workspace_unassign_session: { method: 'POST', path: '/api/v1/workspaces/unassign',
                         body: (a) => ({ session_id: arg(a, 'sessionId', 'session_id') }) },

  // -- Diagnostics --
  diagnostics_get_by_session: { method: 'GET', path: pathWith('/api/v1/diagnostics/sessions/{sessionId}', 'sessionId'),
                         query: (a) => a.limit ? { limit: String(a.limit) } : {} },
  diagnostics_get_subagent_history: { method: 'GET', path: '/api/v1/diagnostics/subagents',
                         query: (a) => a.limit ? { limit: String(a.limit) } : {} },
  diagnostics_clear_by_session: { method: 'DELETE', path: pathWith('/api/v1/diagnostics/sessions/{sessionId}', 'sessionId') },
  diagnostics_clear_all: { method: 'DELETE', path: '/api/v1/diagnostics' },

  // -- Observability --
  observability_snapshot: { method: 'GET', path: '/api/v1/observability/snapshot' },
  observability_history: { method: 'GET', path: '/api/v1/observability/history',
                         query: (a) => {
                           const q: Record<string, string> = {};
                           const since = arg(a, 'since', 'from');
                           const until = arg(a, 'until', 'to');
                           if (since) q.since = String(since);
                           if (until) q.until = String(until);
                           return q;
                         }},
  memory_stats:        { method: 'GET', path: '/api/v1/memory-stats' },

  // -- Background tasks --
  background_task_list: { method: 'GET', path: backgroundTaskPath() },
  background_task_poll: { method: 'POST',
                         path: backgroundTaskPath('poll'),
                         body: (a) => compactBody({
                           yield_time_ms: arg(a, 'yieldTimeMs', 'yield_time_ms'),
                           max_output_bytes: arg(a, 'maxOutputBytes', 'max_output_bytes'),
                         }) },
  background_task_write: { method: 'POST',
                         path: backgroundTaskPath('write'),
                         body: (a) => compactBody({
                           input: a.input,
                           yield_time_ms: arg(a, 'yieldTimeMs', 'yield_time_ms'),
                           max_output_bytes: arg(a, 'maxOutputBytes', 'max_output_bytes'),
                         }) },
  background_task_kill: { method: 'POST',
                         path: backgroundTaskPath('kill'),
                         body: (a) => compactBody({
                           yield_time_ms: arg(a, 'yieldTimeMs', 'yield_time_ms'),
                           max_output_bytes: arg(a, 'maxOutputBytes', 'max_output_bytes'),
                         }) },

  // -- Rewind --
  rewind_list_points:  { method: 'GET',  path: pathWith('/api/v1/rewind/{sessionId}/points', 'sessionId') },
  rewind_execute:      { method: 'POST', path: pathWith('/api/v1/rewind/{sessionId}/execute', 'sessionId'),
                         body: rewindTargetBody },
  rewind_restore_files: { method: 'POST', path: pathWith('/api/v1/rewind/{sessionId}/restore-files', 'sessionId'),
                         body: rewindTargetBody },

  // -- Attachments --
  attachment_read_files: { method: 'POST', path: '/api/v1/attachments/read', body: id },

  // -- GUI-only localStorage or lifecycle commands in HTTP mode --
  config_get_gui:      { method: 'GET', path: '/__gui_config__' },
  config_set_gui:      { method: 'PUT', path: '/__gui_config__' },
  show_window:         { method: 'GET', path: '/__noop__' },
  toggle_devtools:     { method: 'GET', path: '/__noop__' },
  window_set_decorations: { method: 'GET', path: '/__noop__' },
  window_minimize:     { method: 'GET', path: '/__noop__' },
  window_toggle_maximize: { method: 'GET', path: '/__noop__' },
  window_close:        { method: 'GET', path: '/__noop__' },
  window_set_theme:    { method: 'GET', path: '/__noop__' },
};
