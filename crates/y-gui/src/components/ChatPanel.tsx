import { useRef, useEffect, useCallback } from 'react';
import { Sparkles, AlertTriangle } from 'lucide-react';
import type { Message } from '../types';
import type { ToolResultRecord } from '../hooks/useChat';
import { MessageBubble } from './MessageBubble';
import { RestoreDivider } from './RestoreDivider';
import { ContextResetDivider } from './ContextResetDivider';
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
  tombstonedSegments?: TombstonedSegment[];
  onRestoreBranch?: (checkpointId: string) => void;
  toolResults?: ToolResultRecord[];
  contextResetPoints?: number[];
}

/** Threshold in pixels from the bottom to consider the user "at bottom". */
const AUTO_SCROLL_THRESHOLD = 150;

export function ChatPanel({ messages, isStreaming, isLoading, error, onEditMessage, onUndoMessage, onResendMessage, tombstonedSegments, onRestoreBranch, toolResults, contextResetPoints }: ChatPanelProps) {
  const endRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  /** Whether the user is near the bottom of the scroll area. */
  const isNearBottomRef = useRef(true);
  /** Track previous message count to detect new messages (vs. streaming updates). */
  const prevMessageCountRef = useRef(0);

  // Track scroll position to determine if user is near the bottom.
  const handleScroll = useCallback(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    isNearBottomRef.current = distanceFromBottom <= AUTO_SCROLL_THRESHOLD;
  }, []);

  // Smart auto-scroll: only scroll to bottom if the user is already near the bottom.
  // Always scroll when a new message is added (message count changes),
  // but during streaming updates (content growing), only scroll if near bottom.
  // Also scroll when contextResetPoints changes (divider appears/disappears).
  useEffect(() => {
    const messageCountChanged = messages.length !== prevMessageCountRef.current;
    prevMessageCountRef.current = messages.length;

    if (messageCountChanged || isNearBottomRef.current) {
      endRef.current?.scrollIntoView({ behavior: messageCountChanged ? 'smooth' : 'auto' });
    }
  }, [messages, isStreaming, contextResetPoints]);

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
      <div className="chat-messages" ref={scrollContainerRef} onScroll={handleScroll}>
          {(() => {
            // Build a display list interleaving messages and restore dividers.
            const elements: React.ReactNode[] = [];
            const segmentMap = new Map<number, TombstonedSegment>();
            if (tombstonedSegments) {
              for (const seg of tombstonedSegments) {
                segmentMap.set(seg.insertBeforeIndex, seg);
              }
            }
            messages.forEach((msg, idx) => {
              const seg = segmentMap.get(idx);
              if (seg && onRestoreBranch) {
                elements.push(
                  <RestoreDivider
                    key={`divider-${seg.checkpointId}`}
                    checkpointId={seg.checkpointId}
                    tombstonedCount={seg.count}
                    onRestore={onRestoreBranch}
                  />
                );
              }
              // Insert context reset divider(s) between the last pre-reset message and the first post-reset message.
              if (contextResetPoints && contextResetPoints.length > 0) {
                for (let pi = 0; pi < contextResetPoints.length; pi++) {
                  if (contextResetPoints[pi] === idx) {
                    elements.push(<ContextResetDivider key={`context-reset-divider-${pi}`} />);
                  }
                }
              }
              elements.push(
                <MessageBubble
                  key={msg.id}
                  message={msg}
                  onEdit={(content) => onEditMessage?.(content, msg.id)}
                  onUndo={onUndoMessage}
                  onResend={(content) => onResendMessage?.(content, msg.id)}
                  toolResults={msg.id.startsWith('streaming-') ? toolResults : undefined}
                />
              );
            });
            // Check for a trailing divider (after all messages).
            const trailingSeg = segmentMap.get(messages.length);
            if (trailingSeg && onRestoreBranch) {
              elements.push(
                <RestoreDivider
                  key={`divider-${trailingSeg.checkpointId}`}
                  checkpointId={trailingSeg.checkpointId}
                  tombstonedCount={trailingSeg.count}
                  onRestore={onRestoreBranch}
                />
              );
            }
            // Context reset divider(s) at the end (when reset point equals message count).
            if (contextResetPoints && contextResetPoints.length > 0) {
              for (let pi = 0; pi < contextResetPoints.length; pi++) {
                if (contextResetPoints[pi] >= messages.length) {
                  elements.push(<ContextResetDivider key={`context-reset-divider-${pi}`} />);
                }
              }
            }
            return elements;
          })()}

        {isStreaming && !messages.some((m) => m.id.startsWith('streaming-')) && (
          <div className="streaming-indicator">
            <div className="typing-dots">
              <span />
              <span />
              <span />
            </div>
            <span className="streaming-text">Thinking...</span>
          </div>
        )}

        {isStreaming && messages.some((m) => m.id.startsWith('streaming-')) && (
          <div className="streaming-cursor" />
        )}

        {error && (
          <div className="chat-error">
            <span className="error-icon"><AlertTriangle size={14} /></span>
            <span className="error-text">{error}</span>
          </div>
        )}

        <div ref={endRef} />
      </div>
    </div>
  );
}
