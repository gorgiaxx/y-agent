// TypeScript types for the y-agent GUI frontend.

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

export interface SessionInfo {
  id: string;
  title: string | null;
  created_at: string;
  updated_at: string;
  message_count: number;
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
  model?: string;
  provider_id?: string;
  tokens?: { input: number; output: number };
  cost?: number;
  context_window?: number;
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
  content: string;
  model: string;
  provider_id?: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  tool_calls: ToolCallInfo[];
  iterations: number;
  context_window: number;
}

export interface ToolCallInfo {
  name: string;
  success: boolean;
  duration_ms: number;
}

export interface ChatErrorPayload {
  run_id: string;
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
  /** Serialised messages sent to the LLM (first 1 000 chars). */
  prompt_preview?: string;
  /** Assistant text returned by the LLM. */
  response_text?: string;
}

export interface ToolResultEvent {
  type: 'tool_result';
  name: string;
  success: boolean;
  duration_ms: number;
  result_preview: string;
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
}

export type TurnEvent = LlmResponseEvent | ToolResultEvent | LoopLimitEvent | UserMessageEvent | StreamDeltaEvent;

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
}

/** Last-turn metadata cached per session by the backend (from `session_last_turn_meta`). */
export interface TurnMeta {
  provider_id: string | null;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  context_window: number;
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
}
