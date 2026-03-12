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
  tokens?: { input: number; output: number };
  cost?: number;
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

export interface ChatCompletePayload {
  run_id: string;
  content: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  tool_calls: ToolCallInfo[];
  iterations: number;
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
// System
// ---------------------------------------------------------------------------

export interface SystemStatus {
  version: string;
  healthy: boolean;
  provider_count: number;
  session_count: number | null;
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
