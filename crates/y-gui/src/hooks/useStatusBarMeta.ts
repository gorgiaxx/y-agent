// ---------------------------------------------------------------------------
// useStatusBarMeta -- status bar metadata management.
//
// Extracted from App.tsx. Manages the token/cost/model metadata shown
// in the StatusBar component. Multiple sources feed into this state:
//
//   1. Session switch -- restored from backend via `session_last_turn_meta`
//   2. chat:complete event -- authoritative source after each turn
//   3. Live diagnostics -- real-time token updates during streaming
//   4. Message fallback -- extracted from loaded messages (page reload)
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef, startTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Message, TurnMeta, ChatCompletePayload, DiagnosticsEntry } from '../types';

export interface StatusBarMeta {
  provider?: string;
  providerId?: string;
  tokens?: { input: number; output: number };
  cost?: number;
  contextWindow?: number;
  contextTokensUsed?: number;
}

interface UseStatusBarMetaParams {
  activeSessionId: string | null;
  messages: Message[];
  isStreaming: boolean;
  isLoadingMessages: boolean;
  /** Diagnostics entries for the active session. */
  diagnosticEntries: DiagnosticsEntry[];
  /** Whether a diagnostics run is active. */
  isDiagnosticsActive: boolean;
}

export function useStatusBarMeta({
  activeSessionId,
  messages,
  isStreaming,
  isLoadingMessages,
  diagnosticEntries,
  isDiagnosticsActive,
}: UseStatusBarMetaParams): StatusBarMeta {
  const [meta, setMeta] = useState<StatusBarMeta>({});

  // Track last response metadata for status bar.
  const applyMeta = useCallback((turnMeta: TurnMeta | null) => {
    startTransition(() => {
      if (turnMeta) {
        setMeta({
          provider: turnMeta.model || turnMeta.provider_id || undefined,
          providerId: turnMeta.provider_id || undefined,
          tokens: {
            input: turnMeta.context_tokens_used || turnMeta.input_tokens,
            output: turnMeta.output_tokens,
          },
          cost: turnMeta.cost_usd,
          contextWindow: turnMeta.context_window,
          contextTokensUsed: turnMeta.context_tokens_used,
        });
      } else {
        setMeta({});
      }
    });
  }, []);

  // On session switch: restore from backend-cached metadata.
  useEffect(() => {
    if (!activeSessionId) {
      applyMeta(null);
      return;
    }
    invoke<TurnMeta | null>('session_last_turn_meta', { sessionId: activeSessionId })
      .then(applyMeta)
      .catch(() => applyMeta(null));
  }, [activeSessionId, applyMeta]);

  // Listen directly to chat:complete events for status bar meta.
  // This is the authoritative source -- fires once per turn completion with
  // all fields already resolved.
  const activeSessionIdRef = useRef(activeSessionId);
  activeSessionIdRef.current = activeSessionId;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<ChatCompletePayload>('chat:complete', (e) => {
      const payload = e.payload;
      // Only update if the event belongs to the currently viewed session.
      if (payload.session_id !== activeSessionIdRef.current) return;
      startTransition(() => {
        setMeta({
          provider: payload.model || payload.provider_id || undefined,
          providerId: payload.provider_id || undefined,
          tokens: {
            input: payload.context_tokens_used || payload.input_tokens,
            output: payload.output_tokens,
          },
          cost: payload.cost_usd,
          contextWindow: payload.context_window,
          contextTokensUsed: payload.context_tokens_used,
        });
      });
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  // Live update: when diagnostics entries change during an active run,
  // extract the latest llm_response and update the status bar so the
  // token occupancy reflects each iteration in real time.
  useEffect(() => {
    if (!isDiagnosticsActive) return;
    // Find the last llm_response entry from the root agent only.
    for (let i = diagnosticEntries.length - 1; i >= 0; i--) {
      const ev = diagnosticEntries[i].event;
      if (ev.type === 'llm_response' && (!ev.agent_name || ev.agent_name === 'chat-turn')) {
        startTransition(() => {
          setMeta((prev) => ({
            ...prev,
            provider: ev.model || prev.provider,
            tokens: { input: ev.input_tokens, output: ev.output_tokens },
            cost: (prev.cost ?? 0) > ev.cost_usd ? prev.cost : ev.cost_usd,
            contextTokensUsed: ev.input_tokens,
            contextWindow: ev.context_window || prev.contextWindow,
          }));
        });
        break;
      }
    }
  }, [diagnosticEntries, isDiagnosticsActive]);

  // Fallback: extract status bar meta from loaded messages (session switch,
  // page reload). Only runs if there are backend-loaded messages that
  // aren't streaming placeholders.
  // Guarded: skip while streaming or loading.
  useEffect(() => {
    if (isStreaming || isLoadingMessages) return;

    const lastAssistant = [...messages].reverse().find(
      (m) => m.role === 'assistant' && !m.id?.startsWith('streaming-'),
    );
    if (!lastAssistant) return;

    const msgMeta = lastAssistant.metadata as Record<string, unknown> | undefined;
    const usage = msgMeta?.usage as Record<string, unknown> | undefined;
    const providerId = (msgMeta?.provider_id as string | undefined);
    const model = lastAssistant.model
      || (msgMeta?.model as string | undefined)
      || providerId;
    const tokens = lastAssistant.tokens
      || (msgMeta?.input_tokens != null && msgMeta?.output_tokens != null
        ? { input: msgMeta.input_tokens as number, output: msgMeta.output_tokens as number }
        : undefined)
      || (usage?.input_tokens != null && usage?.output_tokens != null
        ? { input: usage.input_tokens as number, output: usage.output_tokens as number }
        : undefined);
    const cost = lastAssistant.cost ?? (msgMeta?.cost_usd as number | undefined);
    const contextWindow = lastAssistant.context_window ?? (msgMeta?.context_window as number | undefined);
    const contextTokensUsed = (msgMeta?.context_tokens_used as number | undefined);

    if (model || tokens || cost != null || contextWindow != null) {
      setMeta((prev) => ({
        provider: model || undefined,
        providerId: providerId || prev.providerId,
        tokens: tokens && contextTokensUsed
          ? { input: contextTokensUsed, output: tokens.output }
          : tokens,
        cost,
        contextWindow: contextWindow ?? undefined,
        contextTokensUsed: contextTokensUsed ?? undefined,
      }));
    }
  }, [messages, isStreaming, isLoadingMessages]);

  return meta;
}
