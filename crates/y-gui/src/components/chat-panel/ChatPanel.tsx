import { Fragment, useRef, useCallback, useLayoutEffect, useMemo, useState, memo, type UIEvent } from 'react';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import { Sparkles, AlertTriangle, ChevronDown, RefreshCw } from 'lucide-react';
import type { Message } from '../../types';
import type { ToolResultRecord } from '../../hooks/chatStreamTypes';
import type { CompactInfo } from '../../hooks/useChat';
import { isLiveStreamingAssistantMessage } from '../../hooks/chatStreamingMessages';
import { UserBubble } from './chat-box/UserBubble';
import { AssistantBubble } from './chat-box/AssistantBubble';
import type { InterleavedSegment } from '../../hooks/useInterleavedSegments';
import { RestoreDivider } from './chat-box/RestoreDivider';
import { ContextResetDivider } from './chat-box/ContextResetDivider';
import { CompactDivider } from './chat-box/CompactDivider';
import { isSteerMessage, steerRunEnd, mergeSteeredTurn } from './steerCoalescing';
import { ChatSearchToolbar } from './ChatSearchToolbar';
import { useChatSearchContext } from '../../hooks/useChatSearchContext';
import {
  INITIAL_CHAT_SCROLL_STATE,
  reduceChatScrollState,
  resolveFollowOutputBehavior,
  resolveFollowScrollTop,
  resolveInitialTopMostItemIndex,
  shouldShowScrollToBottomButton,
  type ChatScrollState,
} from './chatAutoScroll';
import './ChatPanel.css';

/** A tombstoned segment for rendering restore dividers. */
export interface TombstonedSegment {
  checkpointId: string;
  count: number;
  /** Index in the active message list where this divider should appear (before this index). */
  insertBeforeIndex: number;
}

interface ChatPanelProps {
  messages: Message[];
  isStreaming: boolean;
  isLoading: boolean;
  error: string | null;
  onEditMessage?: (content: string, messageId: string) => void;
  onUndoMessage?: (messageId: string) => void;
  onResendMessage?: (content: string, messageId: string) => void;
  /** Retry a turn that ended in a provider error (thaws frozen providers first). */
  onRetryTurn?: (content: string, messageId: string) => void;
  onForkMessage?: (messageIndex: number) => void;
  tombstonedSegments?: TombstonedSegment[];
  onRestoreBranch?: (checkpointId: string) => void;
  toolResults?: ToolResultRecord[];
  /** Getter for event-ordered stream segments (from useChat ref). */
  getStreamSegments?: () => InterleavedSegment[] | null;
  contextResetPoints?: number[];
  onUndoContextReset?: (pointIndex: number) => void;
  compactPoints?: CompactInfo[];
}

// ---------------------------------------------------------------------------
// Display item types -- flat list consumed by Virtuoso
// ---------------------------------------------------------------------------

type DisplayItem =
  | { kind: 'message'; msg: Message; msgIdx: number; toolResults?: ToolResultRecord[] }
  | { kind: 'restore-divider'; segment: TombstonedSegment }
  | { kind: 'context-reset'; pointIndex: number }
  | { kind: 'compact-divider'; info: CompactInfo; pointIndex: number }
  | { kind: 'compact-summary'; info: CompactInfo; pointIndex: number }
  | { kind: 'streaming-indicator' }
  | { kind: 'error'; error: string };

/**
 * Build a flat display-item list by interleaving messages with restore
 * dividers and context-reset dividers. Same logic as the original IIFE
 * but pre-computed for the virtualised list.
 */
function buildDisplayItems(
  messages: Message[],
  tombstonedSegments: TombstonedSegment[] | undefined,
  contextResetPoints: number[] | undefined,
  compactPoints: CompactInfo[] | undefined,
  toolResults: ToolResultRecord[] | undefined,
  isStreaming: boolean,
  error: string | null,
): DisplayItem[] {
  const items: DisplayItem[] = [];
  // The terminal failure bubble is the authoritative display for the current
  // run. Event and persisted errors may use completely different wording, so
  // do not rely on text equality for the final message. Keep text matching for
  // older messages so unrelated load/operation errors still get a banner.
  const terminalStreamError = messages[messages.length - 1]?.metadata?.stream_error;
  const terminalErrorAlreadyRendered = typeof terminalStreamError === 'string'
    && terminalStreamError.length > 0;
  const errorAlreadyRenderedInMessage = error
    ? terminalErrorAlreadyRendered || messages.some((message) => {
      const se = message.metadata?.stream_error;
      if (typeof se !== 'string' || se.length === 0) return false;
      return se === error || error.includes(se) || se.includes(error);
    })
    : false;

  const segmentMap = new Map<number, TombstonedSegment>();
  if (tombstonedSegments) {
    for (const seg of tombstonedSegments) {
      segmentMap.set(seg.insertBeforeIndex, seg);
    }
  }

  const pushDividersAt = (idx: number) => {
    // Restore divider before this message.
    const seg = segmentMap.get(idx);
    if (seg) {
      items.push({ kind: 'restore-divider', segment: seg });
    }

    // Context reset divider(s) at this index.
    if (contextResetPoints) {
      for (let pi = 0; pi < contextResetPoints.length; pi++) {
        if (contextResetPoints[pi] === idx) {
          items.push({ kind: 'context-reset', pointIndex: pi });
        }
      }
    }

    // Compact divider(s) at this index.
    if (compactPoints) {
      for (let pi = 0; pi < compactPoints.length; pi++) {
        if (compactPoints[pi].atIndex === idx) {
          items.push({ kind: 'compact-divider', info: compactPoints[pi], pointIndex: pi });
          if (compactPoints[pi].summary) {
            items.push({ kind: 'compact-summary', info: compactPoints[pi], pointIndex: pi });
          }
        }
      }
    }
  };

  let idx = 0;
  while (idx < messages.length) {
    pushDividersAt(idx);
    const msg = messages[idx];

    // Coalesce a steered assistant turn ([assistant, steer-user, assistant, ...])
    // into a single bubble, so each steer renders as an inline chip at its true
    // injection point -- matching live streaming -- rather than a separate user
    // bubble. Non-steered turns (lone assistant) fall through unchanged.
    if (msg.role === 'assistant' || isSteerMessage(msg)) {
      const { end, sawSteer } = steerRunEnd(messages, idx);
      if (sawSteer) {
        const merged = mergeSteeredTurn(messages.slice(idx, end + 1));
        items.push({ kind: 'message', msg: merged, msgIdx: end });
        idx = end + 1;
        continue;
      }
    }

    // The message itself.
    items.push({
      kind: 'message',
      msg,
      msgIdx: idx,
      toolResults: isLiveStreamingAssistantMessage(msg) ? toolResults : undefined,
    });
    idx++;
  }

  // Trailing restore divider (after all messages).
  const trailingSeg = segmentMap.get(messages.length);
  if (trailingSeg) {
    items.push({ kind: 'restore-divider', segment: trailingSeg });
  }

  // Context reset divider(s) at the end.
  if (contextResetPoints) {
    for (let pi = 0; pi < contextResetPoints.length; pi++) {
      if (contextResetPoints[pi] >= messages.length) {
        items.push({ kind: 'context-reset', pointIndex: pi });
      }
    }
  }

  // Compact divider(s) at the end.
  if (compactPoints) {
    for (let pi = 0; pi < compactPoints.length; pi++) {
      if (compactPoints[pi].atIndex >= messages.length) {
        items.push({ kind: 'compact-divider', info: compactPoints[pi], pointIndex: pi });
        if (compactPoints[pi].summary) {
          items.push({ kind: 'compact-summary', info: compactPoints[pi], pointIndex: pi });
        }
      }
    }
  }

  if (isStreaming && !messages.some((m) => m.id.startsWith('streaming-'))) {
    items.push({ kind: 'streaming-indicator' });
  }

  if (error && !errorAlreadyRenderedInMessage) {
    items.push({ kind: 'error', error });
  }

  return items;
}

/** Nearest preceding user message before `beforeIdx`, for retrying a failed turn. */
function findPrecedingUserMessage(messages: Message[], beforeIdx: number): Message | null {
  for (let i = Math.min(beforeIdx, messages.length) - 1; i >= 0; i--) {
    if (messages[i].role === 'user') return messages[i];
  }
  return null;
}

function getDisplayItemKey(item: DisplayItem): string {
  if (item.kind === 'message') return item.msg.id;
  if (item.kind === 'restore-divider') return `restore-${item.segment.checkpointId}`;
  if (item.kind === 'context-reset') return `reset-${item.pointIndex}`;
  if (item.kind === 'compact-divider') return `compact-div-${item.pointIndex}`;
  if (item.kind === 'compact-summary') return `compact-sum-${item.pointIndex}`;
  if (item.kind === 'streaming-indicator') return 'streaming-indicator';
  if (item.kind === 'error') return 'error';
  return 'unknown';
}

// Below this item count, render a flat list (no Virtuoso overhead).
// Virtualization only pays off for long conversations; short lists render
// faster without it. This also enables SSR/static-markup tests.
const VIRTUALIZATION_THRESHOLD = 50;

function ChatPanelInner({
  messages,
  isStreaming,
  isLoading,
  error,
  onEditMessage,
  onUndoMessage,
  onResendMessage,
  onRetryTurn,
  onForkMessage,
  tombstonedSegments,
  onRestoreBranch,
  toolResults,
  getStreamSegments,
  contextResetPoints,
  onUndoContextReset,
  compactPoints,
}: ChatPanelProps) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const listContentRef = useRef<HTMLDivElement>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const [scrollState, setScrollState] = useState<ChatScrollState>(INITIAL_CHAT_SCROLL_STATE);
  const scrollStateRef = useRef<ChatScrollState>(INITIAL_CHAT_SCROLL_STATE);
  const firstMessageIdRef = useRef<string | null | undefined>(undefined);
  const searchCtx = useChatSearchContext();
  const isSearching = searchCtx.isOpen && !!searchCtx.query;


  // Pre-compute the flat display item list.
  const displayItems = useMemo(
    () => buildDisplayItems(messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error),
    [messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error],
  );
  const useFlatList = isSearching || displayItems.length <= VIRTUALIZATION_THRESHOLD;

  const updateScrollState = useCallback((next: ChatScrollState) => {
    scrollStateRef.current = next;
    setScrollState((current) => (
      current.isAtBottom === next.isAtBottom
        && current.shouldAutoScroll === next.shouldAutoScroll
        ? current
        : next
    ));
  }, []);

  // --- Search-mode (non-virtualized) scroll management ---
  // When search is active, all items render in the DOM so querySelectorAll
  // can find [data-search-match] elements. The manual scroller is used only
  // in that path; Virtuoso owns the scroller otherwise.
  const scrollNativeListToBottom = useCallback(() => {
    const scroller = scrollContainerRef.current;
    if (!scroller) {
      return;
    }

    const nextScrollTop = resolveFollowScrollTop({
      shouldAutoScroll: true,
      scrollHeight: scroller.scrollHeight,
      clientHeight: scroller.clientHeight,
    });
    if (nextScrollTop !== null) {
      scroller.scrollTop = nextScrollTop;
    }
  }, []);

  const syncNativeScrollPosition = useCallback(() => {
    const scroller = scrollContainerRef.current;
    if (!scroller) {
      return;
    }

    const nextScrollTop = resolveFollowScrollTop({
      shouldAutoScroll: scrollStateRef.current.shouldAutoScroll,
      scrollHeight: scroller.scrollHeight,
      clientHeight: scroller.clientHeight,
    });
    if (nextScrollTop !== null) {
      scroller.scrollTop = nextScrollTop;
    }
  }, []);

  useLayoutEffect(() => {
    if (!useFlatList) return;
    const firstMessageId = messages[0]?.id ?? null;
    if (firstMessageIdRef.current !== firstMessageId) {
      firstMessageIdRef.current = firstMessageId;
      scrollStateRef.current = INITIAL_CHAT_SCROLL_STATE;
      scrollNativeListToBottom();
      return;
    }

    syncNativeScrollPosition();
  }, [displayItems, messages, useFlatList, scrollNativeListToBottom, syncNativeScrollPosition]);

  useLayoutEffect(() => {
    if (!useFlatList) return;
    const content = listContentRef.current;
    if (!content || typeof ResizeObserver === 'undefined') {
      return;
    }

    const observer = new ResizeObserver(() => {
      syncNativeScrollPosition();
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [useFlatList, syncNativeScrollPosition]);

  const handleScroll = useCallback((event: UIEvent<HTMLDivElement>) => {
    updateScrollState(
      reduceChatScrollState(scrollStateRef.current, {
        type: 'viewport-scroll',
        metrics: {
          scrollHeight: event.currentTarget.scrollHeight,
          scrollTop: event.currentTarget.scrollTop,
          clientHeight: event.currentTarget.clientHeight,
        },
      }),
    );
  }, [updateScrollState]);

  // Reset scroll state on session switch (Virtuoso mode).
  useLayoutEffect(() => {
    if (useFlatList) return;
    const firstMessageId = messages[0]?.id ?? null;
    if (firstMessageIdRef.current !== firstMessageId) {
      firstMessageIdRef.current = firstMessageId;
      scrollStateRef.current = INITIAL_CHAT_SCROLL_STATE;
      virtuosoRef.current?.scrollToIndex({ index: 'LAST' });
    }
  }, [messages, useFlatList]);

  // --- Virtuoso follow-output + at-bottom detection ---
  const followOutput = useCallback(
    (atBottom: boolean) => resolveFollowOutputBehavior({
      shouldAutoScroll: scrollStateRef.current.shouldAutoScroll,
      isAtBottom: atBottom,
    }),
    [],
  );

  const handleAtBottomStateChange = useCallback((atBottom: boolean) => {
    updateScrollState(
      reduceChatScrollState(scrollStateRef.current, {
        type: 'at-bottom-change',
        isAtBottom: atBottom,
      }),
    );
  }, [updateScrollState]);

  const scrollToBottom = useCallback(() => {
    updateScrollState(reduceChatScrollState(scrollStateRef.current, { type: 'jump-to-bottom' }));
    if (useFlatList) {
      scrollNativeListToBottom();
    } else {
      virtuosoRef.current?.scrollToIndex({ index: 'LAST', behavior: 'smooth' });
    }
  }, [useFlatList, scrollNativeListToBottom, updateScrollState]);

  const showScrollToBottom = shouldShowScrollToBottomButton(scrollState, isStreaming);

  // Render a single display item.
  const renderItem = useCallback((_index: number, item: DisplayItem) => {
    switch (item.kind) {
      case 'restore-divider':
        return onRestoreBranch ? (
          <RestoreDivider
            checkpointId={item.segment.checkpointId}
            tombstonedCount={item.segment.count}
            onRestore={onRestoreBranch}
          />
        ) : null;

      case 'context-reset':
        return (
          <ContextResetDivider
            onUndo={onUndoContextReset
              ? () => onUndoContextReset(item.pointIndex)
              : undefined}
          />
        );

      case 'compact-divider':
        return (
          <CompactDivider
            messagesPruned={item.info.messagesPruned}
            messagesCompacted={item.info.messagesCompacted}
            tokensSaved={item.info.tokensSaved}
          />
        );

      case 'compact-summary':
        return (
          <AssistantBubble
            message={{
              id: `compact-summary-${item.pointIndex}`,
              role: 'assistant',
              content: item.info.summary,
              timestamp: new Date().toISOString(),
              tool_calls: [],
            }}
          />
        );

      case 'streaming-indicator':
        return (
          <div className="streaming-indicator">
            <div className="typing-dots">
              <span /><span /><span />
            </div>
            <span className="streaming-text">Thinking...</span>
          </div>
        );

      case 'error':
        return (
          <div className="chat-error">
            <span className="error-icon"><AlertTriangle size={14} /></span>
            <span className="error-text">{item.error}</span>
            {onRetryTurn && (() => {
              const lastUser = findPrecedingUserMessage(messages, messages.length);
              return lastUser ? (
                <button
                  type="button"
                  className="chat-error-retry-btn"
                  onClick={() => onRetryTurn(lastUser.content, lastUser.id)}
                  title="Retry this request"
                  aria-label="Retry this request"
                  disabled={isStreaming}
                >
                  <RefreshCw size={13} />
                  <span>Retry</span>
                </button>
              ) : null;
            })()}
          </div>
        );

      case 'message': {
        if (item.msg.role === 'user') {
          return (
            <UserBubble
              message={item.msg}
              onEdit={(content) => onEditMessage?.(content, item.msg.id)}
              onUndo={onUndoMessage}
              onResend={(content) => onResendMessage?.(content, item.msg.id)}
              disabled={isStreaming}
            />
          );
        }
        return (
          <AssistantBubble
            message={item.msg}
            messageIndex={item.msgIdx >= 0 ? item.msgIdx : undefined}
            toolResults={item.toolResults}
            getStreamSegments={getStreamSegments}
            onFork={onForkMessage}
            onRetry={
              onRetryTurn
              && typeof item.msg.metadata?.stream_error === 'string'
              && item.msg.metadata.stream_error !== ''
                ? (() => {
                    const prevUser = findPrecedingUserMessage(messages, item.msgIdx);
                    return prevUser
                      ? () => onRetryTurn(prevUser.content, prevUser.id)
                      : undefined;
                  })()
                : undefined
            }
          />
        );
      }

      default:
        return null;
    }
  }, [isStreaming, messages, onEditMessage, onUndoMessage, onResendMessage, onRetryTurn, onForkMessage, onRestoreBranch, onUndoContextReset, getStreamSegments]);

  if (isLoading) {
    return (
      <div className="chat-panel">
        <div className="chat-skeleton">
          <div className="skeleton-row skeleton-row--short" />
          <div className="skeleton-row skeleton-row--long" />
          <div className="skeleton-row skeleton-row--medium" />
        </div>
      </div>
    );
  }

  if (messages.length === 0 && !isStreaming && !error) {
    return (
      <div className="chat-panel">
        <div className="chat-empty">
          <div className="chat-empty-icon">
            <Sparkles size={32} />
          </div>
          <h3 className="chat-empty-title">Welcome to y-agent</h3>
          <p className="chat-empty-subtitle">
            Start a conversation by typing a message below.
          </p>
        </div>
      </div>
    );
  }


  return (
    <div className="chat-panel">
      <div className="chat-messages" style={{ position: 'relative' }}>
        {useFlatList ? (
          // Flat list: render all items in the DOM. Used for search (so
          // querySelectorAll can locate [data-search-match] elements) and
          // for short conversations (Virtuoso overhead isn't worth it).
          <div
            ref={scrollContainerRef}
            className={isSearching ? 'chat-message-list chat-message-list--searching' : 'chat-message-list'}
            onScroll={handleScroll}
          >
            <div ref={listContentRef} className="chat-message-list-content">
              {displayItems.map((item, index) => (
                <Fragment key={getDisplayItemKey(item)}>
                  {renderItem(index, item)}
                </Fragment>
              ))}
            </div>
          </div>
        ) : (
          <Virtuoso<DisplayItem>
            ref={virtuosoRef}
            data={displayItems}
            computeItemKey={(_index, item) => getDisplayItemKey(item)}
            itemContent={renderItem}
            initialTopMostItemIndex={resolveInitialTopMostItemIndex(displayItems.length)}
            followOutput={followOutput}
            atBottomStateChange={handleAtBottomStateChange}
            atBottomThreshold={24}
            className="chat-message-list"
            scrollerRef={(ref) => {
              if (ref instanceof HTMLElement) {
                scrollContainerRef.current = ref as HTMLDivElement;
              }
            }}
          />
        )}
        <ChatSearchToolbar scrollContainerRef={scrollContainerRef} />
        {showScrollToBottom && (
          <button
            type="button"
            className="chat-scroll-to-bottom"
            aria-label="Scroll to bottom"
            title="Scroll to bottom"
            onClick={scrollToBottom}
          >
            <ChevronDown size={18} aria-hidden="true" />
          </button>
        )}
      </div>
    </div>
  );
}

export const ChatPanel = memo(ChatPanelInner);
