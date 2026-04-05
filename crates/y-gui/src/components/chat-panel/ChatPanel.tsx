import { useRef, useEffect, useCallback, useMemo, memo } from 'react';
import { Virtuoso, type VirtuosoHandle } from 'react-virtuoso';
import { Sparkles, AlertTriangle } from 'lucide-react';
import type { Message } from '../../types';
import type { ToolResultRecord, CompactInfo } from '../../hooks/useChat';
import { UserBubble } from './chat-box/UserBubble';
import { AssistantBubble } from './chat-box/AssistantBubble';
import { RestoreDivider } from './chat-box/RestoreDivider';
import { ContextResetDivider } from './chat-box/ContextResetDivider';
import { CompactDivider } from './chat-box/CompactDivider';
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
  contextResetPoints?: number[];
  compactPoints?: CompactInfo[];
}

// ---------------------------------------------------------------------------
// Display item types -- flat list consumed by Virtuoso
// ---------------------------------------------------------------------------

type DisplayItem =
  | { kind: 'message'; msg: Message; toolResults?: ToolResultRecord[] }
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
  contextResetPoints,
  compactPoints,
}: ChatPanelProps) {
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  /** Whether the user is near the bottom of the scroll area. */
  const isAtBottomRef = useRef(true);
  /** Track previous message count to detect new messages. */
  const prevMessageCountRef = useRef(0);

  // Pre-compute the flat display item list.
  const displayItems = useMemo(
    () => buildDisplayItems(messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error),
    [messages, tombstonedSegments, contextResetPoints, compactPoints, toolResults, isStreaming, error],
  );

  // Auto-scroll on new messages (count changes) or streaming updates (if near bottom).
  useEffect(() => {
    const messageCountChanged = messages.length !== prevMessageCountRef.current;
    prevMessageCountRef.current = messages.length;

    if (messageCountChanged || isAtBottomRef.current) {
      virtuosoRef.current?.scrollToIndex({
        index: displayItems.length - 1,
        behavior: messageCountChanged ? 'smooth' : 'auto',
        align: 'end',
      });
    }
  }, [messages.length, displayItems.length]);

  const handleAtBottomStateChange = useCallback((atBottom: boolean) => {
    isAtBottomRef.current = atBottom;
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
        // Compute the 0-based message index from the display-item list.
        const msgIdx = messages.indexOf(item.msg);
        if (item.msg.role === 'user') {
          return (
            <UserBubble
              message={item.msg}
              messageIndex={msgIdx >= 0 ? msgIdx : undefined}
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
          />
        );
      }

      default:
        return null;
    }
  }, [messages, isStreaming, onEditMessage, onUndoMessage, onResendMessage, onForkMessage, onRestoreBranch]);

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
          atBottomStateChange={handleAtBottomStateChange}
          atBottomThreshold={150}
          overscan={600}
          increaseViewportBy={{ top: 400, bottom: 400 }}
          initialTopMostItemIndex={Math.max(0, displayItems.length - 1)}
          style={{ height: '100%', position: 'absolute', top: 0, left: 0, right: 0, bottom: 0 }}
        />
      </div>
    </div>
  );
}

export const ChatPanel = memo(ChatPanelInner);
