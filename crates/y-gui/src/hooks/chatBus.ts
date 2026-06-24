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
  SteerMessage,
} from '../types';
import {
  applyAwaitingInteraction,
  applyInteractionResolved,
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
  awaitingInteractionRuns: Set<string>;
}

export type ChatBusSubscriber = (event: ChatBusEvent) => void;

export type ChatBusEvent =
  | { type: 'started'; run_id: string; session_id: string }
  | { type: 'awaiting_interaction'; run_id: string; session_id: string }
  | { type: 'interaction_resolved'; run_id: string; session_id: string }
  | { type: 'complete'; payload: ChatCompletePayload }
  | { type: 'error'; payload: ChatErrorPayload }
  | { type: 'stream_delta'; run_id: string; session_id: string; content: string; agent_name?: string }
  | { type: 'stream_reasoning_delta'; run_id: string; session_id: string; content: string; agent_name?: string }
  | { type: 'stream_image_delta'; run_id: string; session_id: string; index: number; mime_type: string; partial_data: string; agent_name?: string }
  | { type: 'stream_image_complete'; run_id: string; session_id: string; index: number; mime_type: string; data: string; agent_name?: string }
  | { type: 'tool_start'; session_id: string; name: string; input_preview: string; agent_name?: string }
  | { type: 'tool_result'; session_id: string; name: string; success: boolean; duration_ms: number; input_preview: string; result_preview: string; url_meta?: string; metadata?: Record<string, unknown> }
  | { type: 'steer_injected'; run_id: string; session_id: string; steer_id: string; text: string }
  | { type: 'steer_queue'; session_id: string; queue: SteerMessage[] }
  | { type: 'heartbeat'; run_id: string; session_id: string };

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

export function markChatRunAwaitingInteraction(runId: string, sessionId: string) {
  Object.assign(
    chatBusState,
    applyAwaitingInteraction(chatBusState, runId, sessionId),
  );
  notifyChatSubscribers({
    type: 'awaiting_interaction',
    run_id: runId,
    session_id: sessionId,
  });
}

export function resolveChatRunInteraction(runId: string, sessionId: string) {
  Object.assign(
    chatBusState,
    applyInteractionResolved(chatBusState, runId, sessionId),
  );
  notifyChatSubscribers({
    type: 'interaction_resolved',
    run_id: runId,
    session_id: sessionId,
  });
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
    const resolvedSessionId =
      e.payload.session_id || chatBusState.runToSession[run_id] || '';
    Object.assign(
      chatBusState,
      applyRunTerminal(chatBusState, run_id, resolvedSessionId),
    );
    const enrichedPayload = { ...e.payload, session_id: resolvedSessionId };
    notifyChatSubscribers({ type: 'complete', payload: enrichedPayload });
  });
  chatUnlistenFns.push(u1);

  const u2 = await transport.listen<ChatErrorPayload>('chat:error', (e) => {
    const { run_id } = e.payload;
    const resolvedSessionId =
      e.payload.session_id || chatBusState.runToSession[run_id] || '';
    Object.assign(
      chatBusState,
      applyRunTerminal(chatBusState, run_id, resolvedSessionId),
    );
    const enrichedPayload = { ...e.payload, session_id: resolvedSessionId };
    notifyChatSubscribers({ type: 'error', payload: enrichedPayload });
  });
  chatUnlistenFns.push(u2);

  const u3 = await transport.listen<ProgressPayload>('chat:progress', (e) => {
    const { run_id, event } = e.payload;
    if (
      event.type === 'user_interaction_request'
      || event.type === 'permission_request'
      || event.type === 'plan_review_request'
    ) {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        markChatRunAwaitingInteraction(run_id, session_id);
      }
    }

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
    } else if (event.type === 'heartbeat') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'heartbeat',
          run_id,
          session_id,
        });
      }
    } else if (event.type === 'steer_injected') {
      const session_id = chatBusState.runToSession[run_id];
      if (session_id) {
        notifyChatSubscribers({
          type: 'steer_injected',
          run_id,
          session_id,
          steer_id: event.steer_id,
          text: event.text,
        });
      }
    }
  });
  chatUnlistenFns.push(u3);

  const u4 = await transport.listen<{ session_id: string; queue: SteerMessage[] }>(
    'chat:steer_queue',
    (e) => {
      const { session_id, queue } = e.payload;
      notifyChatSubscribers({ type: 'steer_queue', session_id, queue: queue ?? [] });
    },
  );
  chatUnlistenFns.push(u4);
}

// Kick off immediately so events are never missed due to mount timing.
initialiseChatBus().catch(console.error);
