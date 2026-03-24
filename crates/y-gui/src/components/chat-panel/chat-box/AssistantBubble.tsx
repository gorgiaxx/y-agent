/**
 * AssistantBubble -- self-contained component for rendering assistant/system messages.
 *
 * Renders:
 *  - left-aligned avatar
 *  - ThinkingBlock (from metadata.reasoning_content or <think> tags)
 *  - markdown content with optional inline tool-call segments
 *  - tool_calls array (legacy structured tool calls)
 *  - ActionBar (Copy / Share / Thumbs up-down) on hover
 *  - footer (timestamp, tokens, cost)
 */

import { useState, useCallback, useMemo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Copy,
  Check,
  Share2,
  ThumbsUp,
  ThumbsDown,
} from 'lucide-react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { ToolCallCard } from './ToolCallCard';
import { ThinkingBlock } from './ThinkingBlock';
import { Avatar, makeMarkdownComponents, MarkdownSegment } from './MessageShared';
import { processStreamContent, type ContentSegment } from '../../../hooks/useStreamContent';
import { useResolvedTheme } from '../../../hooks/useTheme';
import './AssistantBubble.css';


export interface AssistantBubbleProps {
  message: Message;
  /** Tool results from progress events (only provided for streaming messages). */
  toolResults?: ToolResultRecord[];
}


/**
 * Extract `<think>...</think>` tags from message content.
 *
 * Some models (e.g. DeepSeek, QwQ) embed chain-of-thought inside `<think>` tags
 * in the main content rather than sending a separate `reasoning` field.
 *
 * Returns the extracted thinking text and the remaining content with tags stripped.
 * If the closing `</think>` tag is missing, the content after `<think>` is treated
 * as still-streaming thinking content.
 */
function extractThinkTags(content: string): {
  thinkContent: string | null;
  strippedContent: string;
  isThinkingIncomplete: boolean;
} {
  const openTag = '<think>';
  const closeTag = '</think>';

  const openIdx = content.indexOf(openTag);
  if (openIdx < 0) {
    return { thinkContent: null, strippedContent: content, isThinkingIncomplete: false };
  }

  const afterOpen = openIdx + openTag.length;
  const closeIdx = content.indexOf(closeTag, afterOpen);

  if (closeIdx < 0) {
    // The <think> tag is not closed -- still streaming thinking content.
    const thinkContent = content.slice(afterOpen).trim();
    const strippedContent = content.slice(0, openIdx).trim();
    return {
      thinkContent: thinkContent || null,
      strippedContent,
      isThinkingIncomplete: true,
    };
  }

  // Complete <think>...</think> block found.
  const thinkContent = content.slice(afterOpen, closeIdx).trim();
  // Strip the entire <think>...</think> block from the content.
  const strippedContent = (
    content.slice(0, openIdx) + content.slice(closeIdx + closeTag.length)
  ).trim();

  return {
    thinkContent: thinkContent || null,
    strippedContent,
    isThinkingIncomplete: false,
  };
}


/** Action bar shown on hover for assistant / system messages. */
function ActionBar({ content }: { content: string }) {
  const [copied, setCopied] = useState(false);
  const [feedback, setFeedback] = useState<'good' | 'bad' | null>(null);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [content]);

  const handleShare = useCallback(() => {
    if (navigator.share) {
      navigator.share({ text: content }).catch(() => {});
    } else {
      // Fallback: copy to clipboard
      navigator.clipboard.writeText(content);
    }
  }, [content]);

  return (
    <div className="message-actions">
      <button className="action-btn" onClick={handleCopy} title="Copy message">
        {copied ? <Check size={14} /> : <Copy size={14} />}
        <span className="action-label">{copied ? 'Copied' : 'Copy'}</span>
      </button>

      <button className="action-btn" onClick={handleShare} title="Share message">
        <Share2 size={14} />
        <span className="action-label">Share</span>
      </button>

      <span className="action-divider" />

      <button
        className={`action-btn feedback-btn ${feedback === 'good' ? 'active' : ''}`}
        onClick={() => setFeedback(feedback === 'good' ? null : 'good')}
        title="Good response"
      >
        <ThumbsUp size={14} />
      </button>

      <button
        className={`action-btn feedback-btn ${feedback === 'bad' ? 'active' : ''}`}
        onClick={() => setFeedback(feedback === 'bad' ? null : 'bad')}
        title="Bad response"
      >
        <ThumbsDown size={14} />
      </button>
    </div>
  );
}


export function AssistantBubble({ message, toolResults }: AssistantBubbleProps) {
  const isSystem = message.role === 'system';
  const isStreamingMsg = message.id.startsWith('streaming-');

  // Resolve theme for syntax highlighting.
  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const markdownComponents = useMemo(() => makeMarkdownComponents(codeThemeStyle), [codeThemeStyle]);

  // Extract <think> tags from content for models that inline reasoning.
  // Priority: metadata.reasoning_content (from stream_reasoning_delta) takes
  // precedence over <think> tag extraction.
  const thinkTagResult = useMemo(() => {
    // Skip if metadata already has reasoning_content from the backend.
    if (typeof message.metadata?.reasoning_content === 'string') return null;
    return extractThinkTags(message.content);
  }, [message.content, message.metadata?.reasoning_content]);

  // The effective content to render (with <think> tags stripped if present).
  const effectiveContent = thinkTagResult?.strippedContent ?? message.content;

  // Process content to extract text segments and tool call blocks.
  // Applied to ALL assistant messages (streaming AND completed) so that
  // accumulated multi-iteration content with tool_call XML renders properly.
  const streamResult = useMemo(() => {
    // Only process if content might contain tool_call or tool_result XML.
    if (!effectiveContent.includes('<tool_call') && !effectiveContent.includes('<tool_cal')
        && !effectiveContent.includes('<tool_result')) {
      return null;
    }
    return processStreamContent(effectiveContent);
  }, [effectiveContent]);

  // Build the tool results lookup by matching order.
  // Sources: (1) live progress events via toolResults prop, (2) metadata from backend.
  const toolResultsMap = useMemo(() => {
    if (!streamResult) return new Map<number, ToolResultRecord>();

    // Determine the source of tool results: live prop or persisted metadata.
    let results: ToolResultRecord[] | undefined = toolResults;
    if (!results || results.length === 0) {
      // Fallback: extract from message metadata (after session reload).
      const metaResults = message.metadata?.tool_results;
      if (Array.isArray(metaResults)) {
        results = (metaResults as Array<Record<string, unknown>>).map((tr) => ({
          name: String(tr.name ?? ''),
          success: Boolean(tr.success),
          durationMs: Number(tr.duration_ms ?? 0),
          resultPreview: String(tr.result_preview ?? ''),
        }));
      }
    }

    if (!results || results.length === 0) return new Map<number, ToolResultRecord>();

    const map = new Map<number, ToolResultRecord>();
    // Track which result indices have been consumed.
    const consumed = new Set<number>();
    streamResult.segments.forEach((seg, segIdx) => {
      if (seg.type !== 'tool_call') return;

      // Find the first unconsumed result matching this tool name.
      for (let ri = 0; ri < results!.length; ri++) {
        if (consumed.has(ri)) continue;
        if (results![ri].name === seg.toolCall.name) {
          map.set(segIdx, results![ri]);
          consumed.add(ri);
          break;
        }
      }
    });
    return map;
  }, [toolResults, streamResult, message.metadata]);

  /** Render inline content segments (text + tool calls). */
  const renderSegments = (segments: ContentSegment[], hasPending: boolean) => {
    const elements: React.ReactNode[] = [];

    segments.forEach((seg, idx) => {
      if (seg.type === 'text') {
        elements.push(
          <MarkdownSegment key={`text-${idx}`} text={seg.text} components={markdownComponents} />
        );
      } else if (seg.type === 'tool_call') {
        const result = toolResultsMap.get(idx);
        const status = result
          ? (result.success ? 'success' : 'error')
          : (isStreamingMsg ? 'running' : 'success');
        elements.push(
          <ToolCallCard
            key={`tc-${idx}`}
            toolCall={{
              id: `tc-${idx}`,
              name: seg.toolCall.name,
              arguments: seg.toolCall.arguments,
            }}
            status={status}
            result={result?.resultPreview}
            durationMs={result?.durationMs}
          />
        );
      }
    });

    // Show pending indicator when buffering an incomplete tool_call tag.
    if (hasPending && isStreamingMsg) {
      elements.push(
        <div key="pending" className="tool-call-pending">
          <div className="tool-call-pending-dots">
            <span /><span /><span />
          </div>
          <span className="tool-call-pending-text">Calling tool...</span>
        </div>
      );
    }

    return elements;
  };

  return (
    <div className={`message-bubble ${message.role}`}>
      <Avatar role={message.role} />
      <div className="message-body">
        <div className="message-header">
          <span className="message-role">
            {isSystem ? 'System' : 'Assistant'}
          </span>
          {message.model && (
            <span className="message-model">{message.model}</span>
          )}
        </div>

        {/* Reasoning/thinking block at the top of assistant messages */}
        {/* Source 1: metadata.reasoning_content (from stream_reasoning_delta events) */}
        {typeof message.metadata?.reasoning_content === 'string' && (
          <ThinkingBlock
            content={message.metadata.reasoning_content}
            isStreaming={isStreamingMsg && !message.metadata?._reasoningDoneTs}
            durationMs={message.metadata?._reasoningDurationMs as number | undefined}
          />
        )}
        {/* Source 2: <think> tags embedded in message content */}
        {thinkTagResult?.thinkContent && (
          <ThinkingBlock
            content={thinkTagResult.thinkContent}
            isStreaming={isStreamingMsg && thinkTagResult.isThinkingIncomplete}
          />
        )}

        {streamResult ? (
          /* Assistant message with tool_call segments: render inline. */
          <div className="message-content markdown-body">
            {renderSegments(streamResult.segments, streamResult.hasPendingToolCall)}
          </div>
        ) : (
          /* Assistant / system messages without tool calls: plain markdown. */
          <div className="message-content markdown-body">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={markdownComponents}
            >
              {effectiveContent}
            </ReactMarkdown>
          </div>
        )}

        {message.tool_calls.length > 0 && (
          <div className="message-tool-calls">
            {message.tool_calls.map((tc) => (
              <ToolCallCard key={tc.id} toolCall={tc} />
            ))}
          </div>
        )}

        <ActionBar content={message.content} />

        <div className="message-footer">
          <span className="message-time">
            {new Date(message.timestamp).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
            })}
          </span>
          {message.tokens && (
            <span className="message-tokens">
              {message.tokens.input + message.tokens.output} tokens
            </span>
          )}
          {message.cost !== undefined && message.cost > 0 && (
            <span className="message-cost">
              ${message.cost.toFixed(4)}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
