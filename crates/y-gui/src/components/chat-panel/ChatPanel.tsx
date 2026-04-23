import { useRef, useEffect, useCallback, useMemo, memo, type UIEvent } from 'react';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import { Sparkles, AlertTriangle } from 'lucide-react';
import type { Message } from '../../types';
import type { ToolResultRecord } from '../../hooks/chatStreamTypes';
import type { CompactInfo } from '../../hooks/useChat';
import { UserBubble } from './chat-box/UserBubble';
import { AssistantBubble } from './chat-box/AssistantBubble';
import type { InterleavedSegment } from '../../hooks/useInterleavedSegments';
import { RestoreDivider } from './chat-box/RestoreDivider';
import { ContextResetDivider } from './chat-box/ContextResetDivider';
import { CompactDivider } from './chat-box/CompactDivider';
import { isNearBottom, resolveAutoScrollBehavior } from './chatAutoScroll';
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
  onForkMessage?: (messageIndex: number) => void;
  tombstonedSegments?: TombstonedSegment[];
  onRestoreBranch?: (checkpointId: string) => void;
  toolResults?: ToolResultRecord[];
  /** Getter for event-ordered stream segments (from useChat ref). */
  getStreamSegments?: () => InterleavedSegment[] | null;
  contextResetPoints?: number[];
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

  const segmentMap = new Map<number, TombstonedSegment>();
  if (tombstonedSegments) {
    for (const seg of tombstonedSegments) {
      segmentMap.set(seg.insertBeforeIndex, seg);
    }
  }

  messages.forEach((msg, idx) => {
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

    // The message itself.
    const isLive = msg.id.startsWith('streaming-')
      || msg.id.startsWith('cancelled-')
      || msg.id.startsWith('error-');
    items.push({
      kind: 'message',
      msg,
      msgIdx: idx,
      toolResults: isLive ? toolResults : undefined,
    });
  });

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

  if (error) {
    items.push({ kind: 'error', error });
  }

  return items;
}

function ChatPanelInner({
  messages,
  isStreaming,
  isLoading,
  error,
  onEditMessage,
  onUndoMessage,
  onResendMessage,
  onForkMessage,
  tombstonedSegments,
  onRestoreBranch,
  toolResults,
  getStreamSegments,
  contextResetPoints,
  compactPoints,
}: ChatPanelProps) {
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  /** Whether auto-scroll should remain enabled for new output. */
  const shouldAutoScrollRef = useRef(true);
  /** Track display item count so new items can animate while streaming growth stays instant. */
  const prevDisplayItemCountRef = useRef(0);

  // Pre-compute the flat display item list.
  const displayItems = useMemo(
    () => buildDisplayItems(messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error),
    [messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error],
  );

  // Keep following new output only while the user stays near the bottom.
  useEffect(() => {
    const behavior = resolveAutoScrollBehavior({
      shouldAutoScroll: shouldAutoScrollRef.current,
      previousItemCount: prevDisplayItemCountRef.current,
      nextItemCount: displayItems.length,
    });
    prevDisplayItemCountRef.current = displayItems.length;

    if (!behavior) {
      return;
    }

    virtuosoRef.current?.scrollToIndex({
      index: displayItems.length - 1,
      behavior,
      align: 'end',
    });
  }, [displayItems.length, isStreaming]);

  const handleScroll = useCallback((event: UIEvent<HTMLDivElement>) => {
    shouldAutoScrollRef.current = isNearBottom({
      scrollHeight: event.currentTarget.scrollHeight,
      scrollTop: event.currentTarget.scrollTop,
      clientHeight: event.currentTarget.clientHeight,
    });
  }, []);

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
        return <ContextResetDivider />;

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
          </div>
        );

      case 'message': {
        if (item.msg.role === 'user') {
          return (
            <UserBubble
              message={item.msg}
              messageIndex={item.msgIdx >= 0 ? item.msgIdx : undefined}
              onEdit={(content) => onEditMessage?.(content, item.msg.id)}
              onUndo={onUndoMessage}
              onResend={(content) => onResendMessage?.(content, item.msg.id)}
              onFork={onForkMessage}
              disabled={isStreaming}
            />
          );
        }
        return (
          <AssistantBubble
            message={item.msg}
            toolResults={item.toolResults}
            getStreamSegments={getStreamSegments}
          />
        );
      }

      default:
        return null;
    }
  }, [isStreaming, onEditMessage, onUndoMessage, onResendMessage, onForkMessage, onRestoreBranch, getStreamSegments]);

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

  if (messages.length === 0 && !isStreaming) {
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
        <Virtuoso
          ref={virtuosoRef}
          data={displayItems}
          computeItemKey={(_index, item) => {
            if (item.kind === 'message') return item.msg.id;
            if (item.kind === 'restore-divider') return `restore-${item.segment.checkpointId}`;
            if (item.kind === 'context-reset') return `reset-${item.pointIndex}`;
            if (item.kind === 'compact-divider') return `compact-div-${item.pointIndex}`;
            if (item.kind === 'compact-summary') return `compact-sum-${item.pointIndex}`;
            if (item.kind === 'streaming-indicator') return 'streaming-indicator';
            if (item.kind === 'error') return 'error';
            return 'unknown';
          }}
          itemContent={renderItem}
          onScroll={handleScroll}
          overscan={1200}
          increaseViewportBy={{ top: 800, bottom: 800 }}
          initialTopMostItemIndex={Math.max(0, displayItems.length - 1)}
          style={{ height: '100%', position: 'absolute', top: 0, left: 0, right: 0, bottom: 0 }}
        />
      </div>
    </div>
  );
}

export const ChatPanel = memo(ChatPanelInner);
