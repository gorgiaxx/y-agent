/**
 * StreamingBubble -- renders a live-streaming assistant message.
 *
 * Handles:
 *  - Live tool results from progress events (toolResults prop)
 *  - Pending tool-call dots animation while buffering incomplete XML tags
 *  - Content that grows on every render as tokens stream in
 *  - Action Segment: groups intermediate tool calls into a collapsible block
 */

import { useMemo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { ToolCallCard } from './ToolCallCard';
import {
  makeMarkdownComponents,
  MarkdownSegment,
  AssistantMessageShell,
  extractThinkTags,
} from './MessageShared';
import { processStreamContent } from '../../../hooks/useStreamContent';
import { segmentActions } from '../../../hooks/useActionSegment';
import { ActionCard } from './ActionCard';
import { useResolvedTheme } from '../../../hooks/useTheme';


export interface StreamingBubbleProps {
  message: Message;
  /** Tool results from progress events (live streaming). */
  toolResults?: ToolResultRecord[];
}


export function StreamingBubble({ message, toolResults }: StreamingBubbleProps) {
  // Resolve theme for syntax highlighting.
  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const markdownComponents = useMemo(() => makeMarkdownComponents(codeThemeStyle), [codeThemeStyle]);

  // Derive effective content (with <think> tags stripped).
  const effectiveContent = useMemo(() => {
    if (typeof message.metadata?.reasoning_content === 'string') return message.content;
    return extractThinkTags(message.content).strippedContent;
  }, [message.content, message.metadata?.reasoning_content]);

  // Process content to extract text segments and tool call blocks.
  const streamResult = useMemo(() => {
    if (!effectiveContent.includes('<tool_call') && !effectiveContent.includes('<tool_cal')
        && !effectiveContent.includes('<tool_result')) {
      return null;
    }
    return processStreamContent(effectiveContent);
  }, [effectiveContent]);

  // Build the tool results lookup by matching order.
  const toolResultsMap = useMemo(() => {
    if (!streamResult) return new Map<number, ToolResultRecord>();

    const results = toolResults;
    if (!results || results.length === 0) return new Map<number, ToolResultRecord>();

    const map = new Map<number, ToolResultRecord>();
    const consumed = new Set<number>();
    streamResult.segments.forEach((seg, segIdx) => {
      if (seg.type !== 'tool_call') return;
      for (let ri = 0; ri < results.length; ri++) {
        if (consumed.has(ri)) continue;
        if (results[ri].name === seg.toolCall.name) {
          map.set(segIdx, results[ri]);
          consumed.add(ri);
          break;
        }
      }
    });
    return map;
  }, [toolResults, streamResult]);

  // Segment into actions vs conclusion.
  const actionResult = useMemo(() => {
    if (!streamResult) return null;
    return segmentActions(streamResult.segments, streamResult.hasPendingToolCall);
  }, [streamResult]);

  return (
    <AssistantMessageShell message={message} isStreaming={true}>
      {streamResult && actionResult && actionResult.actions.length > 0 ? (
        <>
          {/* Action Segment: intermediate tool calls + text */}
          <ActionCard
            segments={actionResult.actions}
            toolCallCount={actionResult.toolCallCount}
            isStreaming={!actionResult.conclusion}
            hasPendingToolCall={streamResult.hasPendingToolCall}
            toolResultsMap={toolResultsMap}
            markdownComponents={markdownComponents}
          />
          {/* Conclusion: trailing text from final LLM iteration */}
          {actionResult.conclusion && actionResult.conclusion.type === 'text' && (
            <div className="message-content markdown-body">
              <MarkdownSegment
                text={actionResult.conclusion.text}
                components={markdownComponents}
              />
            </div>
          )}
        </>
      ) : streamResult ? (
        <div className="message-content">
          {streamResult.segments.map((seg, idx) => {
            if (seg.type === 'text') {
              return (
                <div key={`text-${idx}`} className="markdown-body">
                  <MarkdownSegment text={seg.text} components={markdownComponents} />
                </div>
              );
            }
            if (seg.type === 'tool_call') {
              const result = toolResultsMap.get(idx);
              const status = result
                ? (result.success ? 'success' : 'error')
                : 'running';
              return (
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
            return null;
          })}
          {streamResult.hasPendingToolCall && (
            <div className="tool-call-pending">
              <div className="tool-call-pending-dots">
                <span /><span /><span />
              </div>
              <span className="tool-call-pending-text">Calling tool...</span>
            </div>
          )}
        </div>
      ) : (
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
    </AssistantMessageShell>
  );
}
