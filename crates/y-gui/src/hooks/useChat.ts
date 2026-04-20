// ---------------------------------------------------------------------------
// useChat -- facade hook that composes sub-hooks for chat functionality.
//
// Architecture (post-decomposition):
// - useChatSessionState: opStatus, pending edits, context/compact points,
//   session switch restoration.
// - useChatMessages: message cache, load/clear, sync helpers.
// - useChatStreaming: ChatBus subscription, stream segments, tool results,
//   safety timeout.
// - useChatOperations: send, cancel, edit, undo, resend, restore.
//
// All sub-hooks share mutable state via ChatSharedRefs.
// ---------------------------------------------------------------------------

import { useRef, useEffect, useMemo, type Dispatch, type SetStateAction } from 'react';
import type { Message } from '../types';
import type { ToolResultRecord } from './chatStreamTypes';
import type { InterleavedSegment } from './useInterleavedSegments';
import { DEFAULT_ROOT_AGENT_NAME } from '../constants/agents';
import type { ChatSharedRefs } from './chatSharedState';
import { useChatSessionState } from './useChatSessionState';
import { useChatMessages } from './useChatMessages';
import { useChatStreaming } from './useChatStreaming';
import { useChatOperations } from './useChatOperations';

// Re-export ChatBusEvent for consumers that need the union type.
export type { ChatBusEvent } from './chatBus';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Pending edit state exposed to InputArea via App.tsx. */
export interface PendingEdit {
  messageId: string;
  content: string;
}

/** Options for the sendMessage call (replaces positional parameters). */
export interface SendMessageOptions {
  message: string;
  sessionId: string;
  providerId?: string;
  skills?: string[];
  knowledgeCollections?: string[];
  thinkingEffort?: import('../types').ThinkingEffort | null;
  attachments?: import('../types').Attachment[];
  planMode?: import('../types').PlanMode;
  mcpMode?: import('../types').McpMode | null;
  mcpServers?: string[];
  requestMode?: import('../types').RequestMode;
}

/** Operation status for guarding concurrent actions. */
export type ChatOpStatus =
  | 'idle'
  | 'sending'
  | 'editing'
  | 'undoing'
  | 'resending'
  | 'restoring'
  | 'compacting';

export interface UseChatReturn {
  messages: Message[];
  isStreaming: boolean;
  isLoadingMessages: boolean;
  streamingSessionIds: Set<string>;
  activeRunId: string | null;
  error: string | null;
  /** Current high-level operation status. */
  opStatus: ChatOpStatus;
  /** Pending edit info (for InputArea banner). */
  pendingEdit: PendingEdit | null;
  /** Tool results from the current streaming run (for inline cards). */
  toolResults: ToolResultRecord[];
  /** Get event-ordered segments for the active streaming session.
   *  Returns null if no tool calls are present. Built from event arrival
   *  order (stream_delta -> text, tool_result -> tool card) so the
   *  interleaving is inherently correct without character offsets. */
  getStreamSegments: () => InterleavedSegment[] | null;
  sendMessage: (opts: SendMessageOptions) => Promise<import('../types').ChatStarted | null>;
  cancelRun: () => Promise<void>;
  loadMessages: (sessionId: string) => Promise<void>;
  clearMessages: () => void;
  /** Enter edit mode: populate input box, show edit banner.
   *  No optimistic truncation -- the UI stays unchanged until send. */
  editMessage: (messageId: string, content: string) => void;
  /** Cancel an in-progress edit (restore original view). */
  cancelEdit: () => void;
  /** Execute edit and resend: undo to checkpoint then send new content.
   *  This is the transactional compound operation called from handleSend. */
  editAndResend: (sessionId: string, newContent: string, providerId?: string, thinkingEffort?: import('../types').ThinkingEffort | null, planMode?: import('../types').PlanMode, requestMode?: import('../types').RequestMode) => Promise<import('../types').ChatStarted | null>;
  /** Undo to a specific message: rolls back all state to before that message was sent. */
  undoToMessage: (sessionId: string, messageId: string) => Promise<import('../types').UndoResult | null>;
  /** Resend: keep user message, remove assistant reply, re-run LLM. */
  resendLastTurn: (sessionId: string, messageId: string, content: string, providerId?: string, thinkingEffort?: import('../types').ThinkingEffort | null, planMode?: import('../types').PlanMode) => Promise<import('../types').ChatStarted | null>;
  /** Restore a tombstoned branch. */
  restoreBranch: (sessionId: string, checkpointId: string) => Promise<import('../types').RestoreResult | null>;
  /** All context reset points for the active session (empty = no resets). */
  contextResetPoints: number[];
  /** Add a new context reset point at the current message position. */
  addContextReset: () => void;
  /** Compaction display items (divider + summary) injected after compaction completes. */
  compactPoints: CompactInfo[];
  /** Add a compaction point at the current message position (called by handleCommand). */
  addCompactPoint: (info: Omit<CompactInfo, 'atIndex'>) => void;
  /** Set the operation status (used by handlers for compacting state). */
  setOp: (status: ChatOpStatus) => void;
}

/** Info about a completed compaction for rendering a divider + summary bubble. */
export interface CompactInfo {
  /** Index in the message list where the divider should appear. */
  atIndex: number;
  messagesPruned: number;
  messagesCompacted: number;
  tokensSaved: number;
  summary: string;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useChat(
  activeSessionId: string | null,
  rootAgentNames: string[] = [DEFAULT_ROOT_AGENT_NAME],
): UseChatReturn {
  // -----------------------------------------------------------------------
  // Shared refs -- created once, passed to all sub-hooks.
  // -----------------------------------------------------------------------

  const activeSessionIdRef = useRef<string | null>(activeSessionId);
  const sessionMessagesRef = useRef(new Map<string, Message[]>());
  const sessionActivityRef = useRef(new Map<string, number>());
  const opStatusMapRef = useRef(new Map<string, ChatOpStatus>());
  const opStatusRef = useRef<ChatOpStatus>('idle');
  const toolResultsRef = useRef(new Map<string, ToolResultRecord[]>());
  const streamSegsRef = useRef(new Map<string, InterleavedSegment[]>());
  const contextResetMapRef = useRef(new Map<string, number[]>());
  const compactMapRef = useRef(new Map<string, CompactInfo[]>());
  const rootAgentNamesRef = useRef<string[]>([DEFAULT_ROOT_AGENT_NAME]);

  useEffect(() => {
    const names = rootAgentNames.filter(Boolean);
    rootAgentNamesRef.current = names.length > 0 ? names : [DEFAULT_ROOT_AGENT_NAME];
  }, [rootAgentNames]);

  // Stable refs object -- all members are useRef results (referentially stable),
  // so this memo never invalidates. This prevents sub-hooks from re-running
  // effects that depend on the container object.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const refs: ChatSharedRefs = useMemo(() => ({
    activeSessionIdRef,
    sessionMessagesRef,
    sessionActivityRef,
    opStatusMapRef,
    opStatusRef,
    toolResultsRef,
    streamSegsRef,
    contextResetMapRef,
    compactMapRef,
    rootAgentNamesRef,
  }), []);

  // -----------------------------------------------------------------------
  // 1. Session state (opStatus, pendingEdit, context/compact points)
  // -----------------------------------------------------------------------

  // We need a temporary no-op for setVisibleToolResults and setError to
  // break the circular dependency. The session state hook does not call them
  // during render -- only inside effects triggered by activeSessionId change,
  // so we can safely wire them up after the messages and streaming hooks
  // are created. We use refs to hold the real setters.
  const setVisibleToolResultsHolder = useRef<Dispatch<SetStateAction<ToolResultRecord[]>>>(() => {});
  const setErrorHolder = useRef<Dispatch<SetStateAction<string | null>>>(() => {});

  const sessionState = useChatSessionState(
    activeSessionId,
    refs,
    // Proxy dispatchers that forward to the real setters once wired up.
    (val) => setVisibleToolResultsHolder.current(val),
    (val) => setErrorHolder.current(val),
  );

  // -----------------------------------------------------------------------
  // 2. Messages (visibleMessages, load, clear, syncVisible)
  // -----------------------------------------------------------------------

  const messages = useChatMessages(
    refs,
    sessionState.setOp,
    sessionState.setPendingEdit,
    sessionState.setContextResetPoints,
  );

  // Wire up the error setter now that messages hook owns it.
  setErrorHolder.current = messages.setError;

  // -----------------------------------------------------------------------
  // 3. Streaming (bus subscription, tool results, safety timeout)
  // -----------------------------------------------------------------------

  const streaming = useChatStreaming(
    refs,
    sessionState.setOp,
    sessionState.setOpForSession,
    messages.syncVisible,
    messages.updateStreamingGeneratedImages,
    messages.setVisibleMessages,
    messages.setError,
    sessionState.markSessionActivity,
  );

  // Wire up the tool results setter now that the streaming hook owns it.
  setVisibleToolResultsHolder.current = streaming.setVisibleToolResults;

  // -----------------------------------------------------------------------
  // 4. Operations (send, cancel, edit, undo, resend, restore)
  // -----------------------------------------------------------------------

  const operations = useChatOperations(
    refs,
    sessionState.setOp,
    messages.setError,
    messages.syncVisible,
    messages.loadMessages,
    sessionState.invalidateStaleContextResets,
    sessionState.markSessionActivity,
    sessionState.pendingEdit,
    sessionState.setPendingEdit,
    streaming.setStreamingSessionIds,
    sessionState.getRequestModeFromMessage,
  );

  // -----------------------------------------------------------------------
  // Derived state
  // -----------------------------------------------------------------------

  const isStreaming = activeSessionId
    ? streaming.streamingSessionIds.has(activeSessionId)
    : false;

  // -----------------------------------------------------------------------
  // Return -- identical to the original UseChatReturn contract.
  // -----------------------------------------------------------------------

  return {
    messages: messages.visibleMessages,
    isStreaming,
    isLoadingMessages: messages.isLoadingMessages,
    streamingSessionIds: streaming.streamingSessionIds,
    activeRunId: streaming.activeRunId,
    error: messages.error,
    opStatus: sessionState.opStatus,
    pendingEdit: sessionState.pendingEdit,
    toolResults: streaming.visibleToolResults,
    getStreamSegments: streaming.getStreamSegments,
    sendMessage: operations.sendMessage,
    cancelRun: operations.cancelRun,
    loadMessages: messages.loadMessages,
    clearMessages: messages.clearMessages,
    editMessage: operations.editMessage,
    cancelEdit: operations.cancelEdit,
    editAndResend: operations.editAndResend,
    undoToMessage: operations.undoToMessage,
    resendLastTurn: operations.resendLastTurn,
    restoreBranch: operations.restoreBranch,
    contextResetPoints: sessionState.contextResetPoints,
    addContextReset: sessionState.addContextReset,
    compactPoints: sessionState.compactPoints,
    addCompactPoint: sessionState.addCompactPoint,
    setOp: sessionState.setOp,
  };
}
