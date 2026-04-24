// ---------------------------------------------------------------------------
// ChatBus -- module-level singleton for Tauri chat event listeners.
//
// Extracted from useChat.ts to improve modularity. Tauri event listeners
// are registered ONCE per application lifetime. React StrictMode may
// mount/unmount the hook multiple times but the Tauri listeners are
// unaffected. State mutations are forwarded to all subscribed hook
// instances via a callback registry.
// ---------------------------------------------------------------------------

import { transport, type UnlistenFn } from '../lib';
import type {
  ChatCompletePayload,
  ChatErrorPayload,
  ChatStartedPayload,
  ProgressPayload,
} from '../types';
import {
  applyRunStarted,
  applyRunTerminal,
  createChatRunState,
} from './chatRunState';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ChatBusState {
  runToSession: Record<string, string>;
  streamingSessions: Set<string>;
  pendingRuns: Set<string>;
}

export type ChatBusSubscriber = (event: ChatBusEvent) => void;

export type ChatBusEvent =
  | { type: 'started'; run_id: string; session_id: string }
  | { type: 'complete'; payload: ChatCompletePayload }
  | { type: 'error'; payload: ChatErrorPayload }
  | { type: 'stream_delta'; run_id: string; session_id: string; content: string; agent_name?: string }
  | { type: 'stream_reasoning_delta'; run_id: string; session_id: string; content: string; agent_name?: string }
  | { type: 'stream_image_delta'; run_id: string; session_id: string; index: number; mime_type: string; partial_data: string; agent_name?: string }
  | { type: 'stream_image_complete'; run_id: string; session_id: string; index: number; mime_type: string; data: string; agent_name?: string }
  | { type: 'tool_start'; session_id: string; name: string; input_preview: string; agent_name?: string }
  | { type: 'tool_result'; session_id: string; name: string; success: boolean; duration_ms: number; input_preview: string; result_preview: string; url_meta?: string; metadata?: Record<string, unknown> };

// ---------------------------------------------------------------------------
// Singleton state
// ---------------------------------------------------------------------------

let chatBusInitialised = false;

export const chatBusState: ChatBusState = createChatRunState();

export const chatBusSubscribers = new Set<ChatBusSubscriber>();
const chatUnlistenFns: UnlistenFn[] = [];

// Track run IDs whose cancel has already been processed to prevent the
// duplicate `chat:error` event (emitted by both `chat_cancel` and the
// spawned LLM task) from re-entering the handler.
export const processedCancelledRuns = new Set<string>();

// ---------------------------------------------------------------------------
// Notification
// ---------------------------------------------------------------------------

function notifyChatSubscribers(event: ChatBusEvent) {
  for (const cb of chatBusSubscribers) {
    cb(event);
  }
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

async function initialiseChatBus() {
  if (chatBusInitialised) return;
  chatBusInitialised = true;

  const u0 = await transport.listen<ChatStartedPayload>('chat:started', (e) => {
    const { run_id, session_id } = e.payload;
    Object.assign(chatBusState, applyRunStarted(chatBusState, run_id, session_id));
    notifyChatSubscribers({ type: 'started', run_id, session_id });
  });
  chatUnlistenFns.push(u0);

  const u1 = await transport.listen<ChatCompletePayload>('chat:complete', (e) => {
    const { run_id } = e.payload;
    Object.assign(
      chatBusState,
      applyRunTerminal(chatBusState, run_id, e.payload.session_id),
    );
    notifyChatSubscribers({ type: 'complete', payload: e.payload });
  });
  chatUnlistenFns.push(u1);

  const u2 = await transport.listen<ChatErrorPayload>('chat:error', (e) => {
    const { run_id } = e.payload;
    Object.assign(
      chatBusState,
      applyRunTerminal(chatBusState, run_id, e.payload.session_id),
    );
    notifyChatSubscribers({ type: 'error', payload: e.payload });
  });
  chatUnlistenFns.push(u2);

  const u3 = await transport.listen<ProgressPayload>('chat:progress', (e) => {
    const { run_id, event } = e.payload;
    if (event.type === 'stream_delta') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'stream_delta',
          run_id,
          session_id,
          content: event.content,
          agent_name: event.agent_name,
        });
      }
    } else if (event.type === 'stream_reasoning_delta') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'stream_reasoning_delta',
          run_id,
          session_id,
          content: event.content,
          agent_name: event.agent_name,
        });
      }
    } else if (event.type === 'stream_image_delta') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'stream_image_delta',
          run_id,
          session_id,
          index: event.index,
          mime_type: event.mime_type,
          partial_data: event.partial_data,
          agent_name: event.agent_name,
        });
      }
    } else if (event.type === 'stream_image_complete') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'stream_image_complete',
          run_id,
          session_id,
          index: event.index,
          mime_type: event.mime_type,
          data: event.data,
          agent_name: event.agent_name,
        });
      }
    } else if (event.type === 'tool_start') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'tool_start',
          session_id,
          name: event.name,
          input_preview: event.input_preview ?? '',
          agent_name: event.agent_name,
        });
      }
    } else if (event.type === 'tool_result') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'tool_result',
          session_id,
          name: event.name,
          success: event.success,
          duration_ms: event.duration_ms,
          input_preview: event.input_preview ?? '',
          result_preview: event.result_preview,
          url_meta: event.url_meta ?? undefined,
          metadata: event.metadata ?? undefined,
        });
      }
    }
  });
  chatUnlistenFns.push(u3);
}

// Kick off immediately so events are never missed due to mount timing.
initialiseChatBus().catch(console.error);
