// TypeScript types for the y-agent GUI frontend.

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

export interface SessionInfo {
  id: string;
  agent_id?: string | null;
  title: string | null;
  manual_title?: string | null;
  created_at: string;
  updated_at: string;
  message_count: number;
  has_custom_prompt?: boolean;
}

export interface SessionPromptConfig {
  system_prompt?: string | null;
  prompt_section_ids: string[];
  template_id?: string | null;
}

export interface UserPromptTemplate {
  id: string;
  name: string;
  description?: string | null;
  system_prompt: string;
  prompt_section_ids: string[];
}

export interface WorkspaceInfo {
  id: string;
  name: string;
  path: string;
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  tool_calls: ToolCallBrief[];
  /** Skill names attached to this user message (if any). */
  skills?: string[];
  model?: string;
  provider_id?: string;
  tokens?: { input: number; output: number };
  cost?: number;
  context_window?: number;
  /** Backend metadata (model info, tool results, usage). */
  metadata?: Record<string, unknown>;
}

export interface GeneratedImage {
  index: number;
  mime_type: string;
  data: string;
}

export interface ToolCallBrief {
  id: string;
  name: string;
  arguments: string;
}

// ---------------------------------------------------------------------------
// Chat events (from Rust backend via Tauri events)
// ---------------------------------------------------------------------------

export interface ChatStarted {
  session_id: string;
  run_id: string;
}

/** Payload from the `chat:started` Tauri event (emitted before progress events). */
export interface ChatStartedPayload {
  run_id: string;
  session_id: string;
}

export interface ChatCompletePayload {
  run_id: string;
  session_id: string;
  content: string;
  model: string;
  provider_id?: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  tool_calls: ToolCallInfo[];
  iterations: number;
  context_window: number;
  context_tokens_used: number;
}

export interface ToolCallInfo {
  name: string;
  success: boolean;
  duration_ms: number;
}

export interface ChatErrorPayload {
  run_id: string;
  session_id: string;
  error: string;
}

// ---------------------------------------------------------------------------
// Turn progress events (real-time diagnostics from service layer)
// ---------------------------------------------------------------------------

export interface LlmResponseEvent {
  type: 'llm_response';
  iteration: number;
  model: string;
  input_tokens: number;
  output_tokens: number;
  duration_ms: number;
  cost_usd: number;
  tool_calls_requested: string[];
  /** Serialised messages sent to the LLM (full JSON payload). */
  prompt_preview?: string;
  /** Assistant text returned by the LLM. */
  response_text?: string;
  /** Context window size of the serving provider (tokens). */
  context_window: number;
  /** Name of the agent that produced this event (e.g. 'chat-turn'). */
  agent_name?: string;
}

export interface ToolStartEvent {
  type: 'tool_start';
  name: string;
  input_preview?: string;
  agent_name?: string;
}

export interface ToolResultEvent {
  type: 'tool_result';
  name: string;
  success: boolean;
  duration_ms: number;
  input_preview?: string;
  result_preview: string;
  /** Name of the agent that executed this tool call. */
  agent_name?: string;
  /** Compact URL metadata (JSON) for Browser/WebFetch tools. */
  url_meta?: string;
  /** Optional structured metadata for presentation layers. */
  metadata?: Record<string, unknown>;
}

export interface LoopLimitEvent {
  type: 'loop_limit_hit';
  iterations: number;
  max_iterations: number;
}

export interface UserMessageEvent {
  type: 'user_message';
  content: string;
}

export interface StreamDeltaEvent {
  type: 'stream_delta';
  /** Incremental text content from the LLM. */
  content: string;
  /** Name of the agent that produced this delta. */
  agent_name?: string;
}

export interface StreamReasoningDeltaEvent {
  type: 'stream_reasoning_delta';
  /** Incremental reasoning/thinking text from the LLM. */
  content: string;
  /** Name of the agent that produced this delta. */
  agent_name?: string;
}

export interface StreamImageDeltaEvent {
  type: 'stream_image_delta';
  index: number;
  mime_type: string;
  partial_data: string;
  agent_name?: string;
}

export interface StreamImageCompleteEvent {
  type: 'stream_image_complete';
  index: number;
  mime_type: string;
  data: string;
  agent_name?: string;
}

export interface LlmErrorEvent {
  type: 'llm_error';
  /** 1-based iteration where the error occurred. */
  iteration: number;
  /** Human-readable error description. */
  error: string;
  /** LLM call wall-clock duration (ms) before the error. */
  duration_ms: number;
  /** Model that was being called. */
  model: string;
  /** Serialised messages sent to the LLM (full JSON payload). */
  prompt_preview: string;
  /** Context window size of the serving provider (tokens). */
  context_window: number;
  /** Name of the agent where the error occurred. */
  agent_name?: string;
}

export type TurnEvent =
  | LlmResponseEvent
  | ToolStartEvent
  | ToolResultEvent
  | LoopLimitEvent
  | UserMessageEvent
  | StreamDeltaEvent
  | StreamReasoningDeltaEvent
  | StreamImageDeltaEvent
  | StreamImageCompleteEvent
  | LlmErrorEvent;

export interface ProgressPayload {
  run_id: string;
  event: TurnEvent;
}

/** A single entry in the diagnostics timeline. */
export interface DiagnosticsEntry {
  id: string;
  timestamp: string;
  event: TurnEvent;
}

// ---------------------------------------------------------------------------
// Gateway broadcast events (from DiagnosticsProviderPool / DiagnosticsToolGateway)
//
// These mirror the Rust `DiagnosticsEvent` enum with
// #[serde(tag = "type", rename_all = "snake_case")].
// ---------------------------------------------------------------------------

export interface DiagLlmCallCompleted {
  type: 'llm_call_completed';
  trace_id: string;
  observation_id: string;
  session_id: string | null;
  agent_name: string;
  iteration: number;
  model: string;
  input_tokens: number;
  output_tokens: number;
  duration_ms: number;
  cost_usd: number;
  tool_calls_requested: string[];
  prompt_preview: string;
  response_text: string;
  context_window: number;
}

export interface DiagLlmCallFailed {
  type: 'llm_call_failed';
  trace_id: string;
  observation_id: string | null;
  session_id: string | null;
  agent_name: string;
  iteration: number;
  model: string;
  error: string;
  duration_ms: number;
}

export interface DiagToolCallCompleted {
  type: 'tool_call_completed';
  trace_id: string;
  session_id: string | null;
  agent_name: string;
  tool_name: string;
  success: boolean;
  duration_ms: number;
  input_preview: string;
  result_preview: string;
}

export interface DiagSubagentCompleted {
  type: 'subagent_completed';
  trace_id: string;
  session_id: string | null;
  agent_name: string;
  success: boolean;
}

export type DiagnosticsGatewayEvent =
  | DiagLlmCallCompleted
  | DiagLlmCallFailed
  | DiagToolCallCompleted
  | DiagSubagentCompleted;

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

export interface SystemStatus {
  version: string;
  healthy: boolean;
  provider_count: number;
  session_count: number | null;
}

/** Summary of a configured LLM provider (from `provider_list` command). */
export interface ProviderInfo {
  id: string;
  model: string;
  provider_type: string;
  capabilities: string[];
}

/** Snapshot of backend in-memory collection sizes (from `memory_stats`). */
export interface MemoryStats {
  pending_runs: number;
  turn_meta_cache: number;
  pruning_watermarks: number;
  session_permission_modes: number;
  pending_interactions: number;
  pending_permissions: number;
  file_history_sessions: number;
  file_history_total_snapshots: number;
}

// ---------------------------------------------------------------------------
// Thinking / Reasoning
// ---------------------------------------------------------------------------

export type ThinkingEffort = 'low' | 'medium' | 'high' | 'max';

/** Plan mode: controls whether planning/loop prompts are injected. */
export type PlanMode = 'fast' | 'auto' | 'plan' | 'loop';

/** MCP mode: controls which MCP server tools are included in requests. */
export type McpMode = 'auto' | 'manual' | 'disabled';

/** High-level provider request mode. */
export type RequestMode = 'text_chat' | 'image_generation';

/** Image generation options (persisted via localStorage). */
export interface ImageGenerationOptions {
  watermark: boolean;
  max_images: number;
  size: string | null;
}

export const DEFAULT_IMAGE_GENERATION_OPTIONS: ImageGenerationOptions = {
  watermark: false,
  max_images: 1,
  size: null,
};

/** Last-turn metadata cached per session by the backend (from `session_last_turn_meta`). */
export interface TurnMeta {
  provider_id: string | null;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  context_window: number;
  context_tokens_used: number;
}

/** Result of a chat undo operation (from `chat_undo`). */
export interface UndoResult {
  messages_removed: number;
  restored_turn_number: number;
  files_restored: number;
}

/** Checkpoint info returned by `chat_checkpoint_list`. */
export interface ChatCheckpointInfo {
  checkpoint_id: string;
  session_id: string;
  turn_number: number;
  message_count_before: number;
  created_at: string;
}

/** Message with active/tombstone status (from `chat_get_messages_with_status`). */
export interface MessageWithStatus {
  id: string;
  role: string;
  content: string;
  status: 'active' | 'tombstone';
  checkpoint_id?: string;
  model?: string;
  input_tokens?: number;
  output_tokens?: number;
  cost_usd?: number;
  context_window?: number;
  created_at: string;
}

/** Result of a branch restoration (from `chat_restore_branch`). */
export interface RestoreResult {
  tombstoned_count: number;
  restored_count: number;
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

export interface GuiConfig {
  theme: 'dark' | 'light' | 'system';
  font_size: number;
  send_on_enter: boolean;
  window_width: number;
  window_height: number;
  setup_completed: boolean;
  translate_target_language: string;
  use_custom_decorations: boolean;
}

// ---------------------------------------------------------------------------
// Observability (live system snapshots)
// ---------------------------------------------------------------------------

/** Point-in-time snapshot of the entire system (from `observability_snapshot`). */
export interface SystemSnapshot {
  timestamp: string;
  providers: ProviderSnapshot[];
  agents: AgentPoolSnapshot;
  scheduler: SchedulerQueueSnapshot | null;
}

/** Per-provider combined state: metadata + freeze + concurrency + metrics. */
export interface ProviderSnapshot {
  id: string;
  model: string;
  provider_type: string;
  tags: string[];
  is_frozen: boolean;
  freeze_reason: string | null;
  max_concurrency: number;
  active_requests: number;
  total_requests: number;
  total_errors: number;
  total_input_tokens: number;
  total_output_tokens: number;
  estimated_cost_usd: number;
  error_rate: number;
}

/** Aggregate agent pool state. */
export interface AgentPoolSnapshot {
  total_instances: number;
  active_instances: number;
  available_slots: number;
  instances: AgentInstanceSnapshot[];
}

/** Per-instance snapshot of a running agent. */
export interface AgentInstanceSnapshot {
  instance_id: string;
  agent_name: string;
  state: string;
  delegation_id: string | null;
  iterations: number;
  tool_calls: number;
  tokens_used: number;
  elapsed_ms: number;
  delegation_depth: number;
}

/** Priority scheduler queue snapshot. */
export interface SchedulerQueueSnapshot {
  active_critical: number;
  active_normal: number;
  active_idle: number;
  total_capacity: number;
  critical_reserve_pct: number;
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

/** Installed skill summary (from `skill_list`). */
export interface SkillInfo {
  name: string;
  description: string;
  version: string;
  tags: string[];
  enabled: boolean;
}

/** Full skill detail (from `skill_get`). */
export interface SkillDetail extends SkillInfo {
  root_content: string;
  author: string | null;
  classification_type: string | null;
  dir_path: string;
}

/** File/directory entry within a skill directory (from `skill_get_files`). */
export interface SkillFileEntry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  children?: SkillFileEntry[];
}

/** Permissions the skill requires, as assessed by the security screening agent. */
export interface PermissionsNeeded {
  files_read: string[];
  files_write: string[];
  network: string[];
  commands: string[];
}

/** Result of a skill import operation (from `skill_import`). */
export interface SkillImportResult {
  decision: 'accepted' | 'optimized' | 'rejected' | 'partial_accept';
  classification: string;
  skill_id: string | null;
  error: string | null;
  security_issues: string[];
  permissions_needed: PermissionsNeeded | null;
}

// ---------------------------------------------------------------------------
// Knowledge
// ---------------------------------------------------------------------------

/** Collection summary (from `kb_collection_list`). */
export interface KnowledgeCollectionInfo {
  id: string;
  name: string;
  description: string;
  entry_count: number;
  chunk_count: number;
  total_bytes: number;
  created_at: string;
}

/** Knowledge entry summary (from `kb_entry_list`). */
export interface KnowledgeEntryInfo {
  id: string;
  title: string;
  source_uri: string;
  source_type: string;
  domains: string[];
  quality_score: number;
  chunk_count: number;
  content_size: number;
  state: 'active' | 'inactive' | 'processing';
  hit_count: number;
  updated_at: string;
  /** Multi-dimensional metadata fields. */
  document_type: string | null;
  industry: string | null;
  subcategory: string | null;
  interpreted_title: string | null;
  tags: string[];
}

/** Entry detail with L0/L1/L2 content (from `kb_entry_detail`). */
export interface KnowledgeEntryDetail {
  id: string;
  title: string;
  source_uri: string;
  domains: string[];
  quality_score: number;
  state: string;
  hit_count: number;
  total_chunk_count: number;
  l0_summary: string;
  l1_sections: KnowledgeSection[];
  l2_chunks: KnowledgeChunk[];
  /** Multi-dimensional metadata fields. */
  document_type: string | null;
  industry: string | null;
  subcategory: string | null;
  interpreted_title: string | null;
  tags: string[];
}

/** A section header+summary (L1 resolution). */
export interface KnowledgeSection {
  index: number;
  title: string;
  summary: string;
}

/** A content chunk (L2 resolution). */
export interface KnowledgeChunk {
  id: string;
  content: string;
  token_estimate: number;
  section_index: number;
}

/** Search result item (from `kb_search`). */
export interface KnowledgeSearchResult {
  chunk_id: string;
  title: string;
  content: string;
  relevance: number;
  domains: string[];
}

/** Ingest result (from `kb_ingest`). */
export interface KnowledgeIngestResult {
  success: boolean;
  entry_id: string | null;
  chunk_count: number;
  domains: string[];
  quality_score: number;
  message: string;
}

/** Ingest progress event payload (from `kb:ingest_progress` event). */
export interface KnowledgeIngestProgress {
  stage: 'fetching' | 'chunking' | 'classifying' | 'indexing' | 'done' | 'error';
  chunk_progress?: { current: number; total: number };
  message: string;
}

/** Global knowledge base stats (from `kb_stats`). */
export interface KnowledgeStats {
  total_collections: number;
  total_entries: number;
  total_chunks: number;
  total_hits: number;
}

// ---------------------------------------------------------------------------
// Attachments (multimodal image uploads)
// ---------------------------------------------------------------------------

/** Attachment data returned by `attachment_read_files`. */
export interface Attachment {
  id: string;
  filename: string;
  mime_type: string;
  base64_data: string;
  size: number;
}
