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
}

function id(args: Record<string, unknown>) {
  return args;
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

// prettier-ignore
export const COMMAND_MAP: Record<string, EndpointDef> = {
  // -- System / Health --
  health_check:        { method: 'GET',  path: '/health' },
  system_status:       { method: 'GET',  path: '/api/v1/status' },
  provider_list:       { method: 'GET',  path: '/api/v1/providers' },
  app_paths:           { method: 'GET',  path: '/api/v1/app-paths' },

  // -- Sessions --
  session_list:        { method: 'GET',  path: '/api/v1/sessions',
                         query: (a) => a.agentId ? { agent_id: String(a.agentId) } : {} },
  session_create:      { method: 'POST', path: '/api/v1/sessions', body: id },
  session_get_messages: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/messages', 'sessionId'),
                         query: (a) => a.last ? { last: String(a.last) } : {} },
  session_delete:      { method: 'DELETE', path: pathWith('/api/v1/sessions/{sessionId}', 'sessionId') },
  session_truncate_messages: { method: 'POST', path: pathWith('/api/v1/sessions/{sessionId}/truncate', 'sessionId'),
                         body: (a) => ({ keep_count: a.keepCount }) },
  session_get_context_reset: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/context-reset', 'sessionId') },
  session_set_context_reset: { method: 'PUT', path: pathWith('/api/v1/sessions/{sessionId}/context-reset', 'sessionId'),
                         body: (a) => ({ index: a.index }) },
  session_get_custom_prompt: { method: 'GET', path: pathWith('/api/v1/sessions/{sessionId}/custom-prompt', 'sessionId') },
  session_set_custom_prompt: { method: 'PUT', path: pathWith('/api/v1/sessions/{sessionId}/custom-prompt', 'sessionId'),
                         body: (a) => ({ prompt: a.prompt }) },
  session_fork:        { method: 'POST', path: pathWith('/api/v1/sessions/{sessionId}/fork', 'sessionId'),
                         body: (a) => ({ at_index: a.atIndex }) },
  session_rename:      { method: 'PUT',  path: pathWith('/api/v1/sessions/{sessionId}/rename', 'sessionId'),
                         body: (a) => ({ title: a.title }) },

  // -- Chat --
  chat_send:           { method: 'POST', path: '/api/v1/chat/send', body: id },
  chat_cancel:         { method: 'POST', path: '/api/v1/chat/cancel', body: (a) => ({ run_id: a.runId }) },
  chat_undo:           { method: 'POST', path: '/api/v1/chat/undo', body: id },
  chat_resend:         { method: 'POST', path: '/api/v1/chat/resend', body: id },
  chat_checkpoint_list: { method: 'GET', path: pathWith('/api/v1/chat/checkpoints/{sessionId}', 'sessionId') },
  chat_find_checkpoint_for_resend: { method: 'POST', path: '/api/v1/chat/find-checkpoint', body: id },
  chat_get_messages_with_status: { method: 'GET', path: pathWith('/api/v1/chat/messages-with-status/{sessionId}', 'sessionId') },
  chat_restore_branch: { method: 'POST', path: '/api/v1/chat/restore-branch', body: id },
  context_compact:     { method: 'POST', path: pathWith('/api/v1/chat/compact/{sessionId}', 'sessionId') },
  chat_answer_question: { method: 'POST', path: '/api/v1/chat/answer-question', body: id },
  chat_answer_permission: { method: 'POST', path: '/api/v1/chat/answer-permission', body: id },
  session_last_turn_meta: { method: 'GET', path: pathWith('/api/v1/chat/last-turn-meta/{sessionId}', 'sessionId') },
  // -- Agents --
  agent_list:          { method: 'GET',  path: '/api/v1/agents' },
  agent_get:           { method: 'GET',  path: pathWith('/api/v1/agents/{id}', 'id') },
  agent_source_get:    { method: 'GET',  path: pathWith('/api/v1/agents/{id}/source', 'id') },
  agent_toml_parse:    { method: 'POST', path: '/api/v1/agents/parse-toml', body: id },
  agent_save:          { method: 'PUT',  path: pathWith('/api/v1/agents/{id}', 'id'),
                         body: bodyWithout('id') },
  agent_reset:         { method: 'POST', path: pathWith('/api/v1/agents/{id}/reset', 'id') },
  agent_reload:        { method: 'POST', path: '/api/v1/agents/reload' },
  agent_tool_list:     { method: 'GET',  path: '/api/v1/agents/tools' },
  agent_prompt_section_list: { method: 'GET', path: '/api/v1/agents/prompt-sections' },
  translate_text:      { method: 'POST', path: '/api/v1/agents/translate', body: id },

  // -- Config --
  config_get:          { method: 'GET',  path: '/api/v1/config' },
  config_set_section:  { method: 'PUT',  path: pathWith('/api/v1/config/{section}', 'section'),
                         body: bodyWithout('section') },
  config_get_section:  { method: 'GET',  path: pathWith('/api/v1/config/{section}', 'section') },
  config_save_section: { method: 'PUT',  path: pathWith('/api/v1/config/{section}', 'section'),
                         body: (a) => ({ content: a.content }) },
  config_reload:       { method: 'POST', path: '/api/v1/config/reload' },
  provider_test:       { method: 'POST', path: '/api/v1/providers/test', body: id },
  provider_list_models: { method: 'POST', path: '/api/v1/providers/list-models', body: id },
  mcp_config_get:      { method: 'GET',  path: '/api/v1/config/mcp' },
  mcp_config_save:     { method: 'PUT',  path: '/api/v1/config/mcp', body: id },
  prompt_list:         { method: 'GET',  path: '/api/v1/config/prompts' },
  prompt_get:          { method: 'GET',  path: pathWith('/api/v1/config/prompts/{filename}', 'filename') },
  prompt_save:         { method: 'PUT',  path: pathWith('/api/v1/config/prompts/{filename}', 'filename'),
                         body: (a) => ({ content: a.content }) },
  prompt_get_default:  { method: 'GET',  path: pathWith('/api/v1/config/prompts/{filename}/default', 'filename') },
  // -- Knowledge --
  kb_collection_list:  { method: 'GET',  path: '/api/v1/knowledge/collections' },
  kb_collection_create: { method: 'POST', path: '/api/v1/knowledge/collections', body: id },
  kb_collection_delete: { method: 'DELETE', path: pathWith('/api/v1/knowledge/collections/{name}', 'name') },
  kb_collection_rename: { method: 'POST', path: pathWith('/api/v1/knowledge/collections/{name}/rename', 'name'),
                         body: (a) => ({ new_name: a.newName }) },
  kb_entry_list:       { method: 'GET',  path: pathWith('/api/v1/knowledge/collections/{collection}/entries', 'collection') },
  kb_entry_detail:     { method: 'GET',  path: pathWith('/api/v1/knowledge/entries/{id}', 'id') },
  kb_entry_delete:     { method: 'DELETE', path: pathWith('/api/v1/knowledge/entries/{id}', 'id') },
  kb_entry_update_metadata: { method: 'PATCH', path: pathWith('/api/v1/knowledge/entries/{id}/metadata', 'id'),
                         body: bodyWithout('id') },
  kb_search:           { method: 'POST', path: '/api/v1/knowledge/search', body: id },
  kb_ingest:           { method: 'POST', path: '/api/v1/knowledge/ingest', body: id },
  kb_ingest_batch:     { method: 'POST', path: '/api/v1/knowledge/ingest-batch', body: id },
  kb_expand_folder:    { method: 'POST', path: '/api/v1/knowledge/expand-folder', body: id },
  kb_stats:            { method: 'GET',  path: '/api/v1/knowledge/stats' },

  // -- Skills --
  skill_list:          { method: 'GET',  path: '/api/v1/skills' },
  skill_get:           { method: 'GET',  path: pathWith('/api/v1/skills/{name}', 'name') },
  skill_uninstall:     { method: 'DELETE', path: pathWith('/api/v1/skills/{name}', 'name') },
  skill_set_enabled:   { method: 'PUT',  path: pathWith('/api/v1/skills/{name}/enabled', 'name'),
                         body: (a) => ({ enabled: a.enabled }) },
  skill_get_files:     { method: 'GET',  path: pathWith('/api/v1/skills/{name}/files', 'name') },
  skill_read_file:     { method: 'GET',  path: (a) => `/api/v1/skills/${encodeURIComponent(String(a.name))}/files/${a.path}` },
  skill_save_file:     { method: 'PUT',  path: (a) => `/api/v1/skills/${encodeURIComponent(String(a.name))}/files/${a.path}`,
                         body: (a) => ({ content: a.content }) },

  // -- Workflows --
  workflow_list:       { method: 'GET',  path: '/api/v1/workflows' },
  workflow_get:        { method: 'GET',  path: pathWith('/api/v1/workflows/{id}', 'id') },
  workflow_create:     { method: 'POST', path: '/api/v1/workflows', body: id },
  workflow_update:     { method: 'PUT',  path: pathWith('/api/v1/workflows/{id}', 'id'), body: bodyWithout('id') },
  workflow_delete:     { method: 'DELETE', path: pathWith('/api/v1/workflows/{id}', 'id') },
  workflow_validate:   { method: 'POST', path: '/api/v1/workflows/validate', body: id },
  workflow_dag:        { method: 'GET',  path: pathWith('/api/v1/workflows/{id}/dag', 'id') },
  workflow_execute:    { method: 'POST', path: pathWith('/api/v1/workflows/{id}/execute', 'id'), body: bodyWithout('id') },
  // -- Schedules --
  schedule_list:       { method: 'GET',  path: '/api/v1/schedules' },
  schedule_get:        { method: 'GET',  path: pathWith('/api/v1/schedules/{id}', 'id') },
  schedule_create:     { method: 'POST', path: '/api/v1/schedules', body: id },
  schedule_update:     { method: 'PUT',  path: pathWith('/api/v1/schedules/{id}', 'id'), body: bodyWithout('id') },
  schedule_delete:     { method: 'DELETE', path: pathWith('/api/v1/schedules/{id}', 'id') },
  schedule_pause:      { method: 'POST', path: pathWith('/api/v1/schedules/{id}/pause', 'id') },
  schedule_resume:     { method: 'POST', path: pathWith('/api/v1/schedules/{id}/resume', 'id') },
  schedule_execution_history: { method: 'GET', path: pathWith('/api/v1/schedules/{id}/executions', 'id') },
  schedule_execution_get: { method: 'GET', path: pathWith('/api/v1/schedules/executions/{executionId}', 'executionId') },
  schedule_trigger_now: { method: 'POST', path: pathWith('/api/v1/schedules/{id}/trigger', 'id') },

  // -- Workspaces --
  workspace_list:      { method: 'GET',  path: '/api/v1/workspaces' },
  workspace_create:    { method: 'POST', path: '/api/v1/workspaces', body: id },
  workspace_update:    { method: 'PUT',  path: pathWith('/api/v1/workspaces/{id}', 'id'), body: bodyWithout('id') },
  workspace_delete:    { method: 'DELETE', path: pathWith('/api/v1/workspaces/{id}', 'id') },
  workspace_session_map: { method: 'GET', path: '/api/v1/workspaces/session-map' },
  workspace_assign_session: { method: 'POST', path: '/api/v1/workspaces/assign', body: id },
  workspace_unassign_session: { method: 'POST', path: '/api/v1/workspaces/unassign', body: id },

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
                           if (a.from) q.from = String(a.from);
                           if (a.to) q.to = String(a.to);
                           return q;
                         }},

  // -- Rewind --
  rewind_list_points:  { method: 'GET',  path: pathWith('/api/v1/rewind/{sessionId}/points', 'sessionId') },
  rewind_execute:      { method: 'POST', path: pathWith('/api/v1/rewind/{sessionId}/execute', 'sessionId'),
                         body: bodyWithout('sessionId') },
  rewind_restore_files: { method: 'POST', path: pathWith('/api/v1/rewind/{sessionId}/restore-files', 'sessionId'),
                         body: bodyWithout('sessionId') },

  // -- Attachments --
  attachment_read_files: { method: 'POST', path: '/api/v1/attachments/read', body: id },

  // -- GUI-only (no-op or localStorage in HTTP mode) --
  config_get_gui:      { method: 'GET', path: '/__gui_config__' },
  config_set_gui:      { method: 'PUT', path: '/__gui_config__' },
  skill_import:        { method: 'POST', path: '/__noop__' },
  show_window:         { method: 'GET', path: '/__noop__' },
  toggle_devtools:     { method: 'GET', path: '/__noop__' },
  window_set_decorations: { method: 'GET', path: '/__noop__' },
  window_minimize:     { method: 'GET', path: '/__noop__' },
  window_toggle_maximize: { method: 'GET', path: '/__noop__' },
  window_close:        { method: 'GET', path: '/__noop__' },
  window_set_theme:    { method: 'GET', path: '/__noop__' },
  skill_open_folder:   { method: 'GET', path: '/__noop__' },
};
