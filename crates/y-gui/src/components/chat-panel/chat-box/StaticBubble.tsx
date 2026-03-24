/**
 * StaticBubble -- renders a completed/history assistant message.
 *
 * Handles:
 *  - Tool results from message.metadata.tool_results (persisted after session reload)
 *  - Content is stable (no streaming), so memoization is straightforward
 *  - Tool call status defaults to 'success' when no result record exists
 *  - Action Segment: groups intermediate tool calls into a collapsible block
 *
 * Two action segment paths:
 *  1. Content contains tool_call XML (just-completed or accumulated content)
 *     -> parse via processStreamContent, then segmentActions
 *  2. Content is plain text but metadata.tool_results has entries (backend history)
 *     -> build action segments from metadata.tool_results
 *     Note: message.tool_calls is EMPTY for persisted assistant messages because
 *     the backend builds the final message with `tool_calls: vec![]`.
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
import { processStreamContent, type ContentSegment } from '../../../hooks/useStreamContent';
import { segmentActions } from '../../../hooks/useActionSegment';
import { ActionCard } from './ActionCard';
import { useResolvedTheme } from '../../../hooks/useTheme';


export interface StaticBubbleProps {
  message: Message;
}


export function StaticBubble({ message }: StaticBubbleProps) {
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
  // Applied to completed messages so that accumulated multi-iteration content
  // with tool_call XML renders properly.
  const streamResult = useMemo(() => {
    if (!effectiveContent.includes('<tool_call') && !effectiveContent.includes('<tool_cal')
        && !effectiveContent.includes('<tool_result')) {
      return null;
    }
    return processStreamContent(effectiveContent);
  }, [effectiveContent]);

  // Parse tool results from persisted metadata (reusable for both paths).
  const metaToolResults = useMemo((): ToolResultRecord[] => {
    const metaResults = message.metadata?.tool_results;
    if (!Array.isArray(metaResults)) return [];
    return (metaResults as Array<Record<string, unknown>>).map((tr) => ({
      name: String(tr.name ?? ''),
      arguments: String(tr.arguments ?? ''),
      success: Boolean(tr.success),
      durationMs: Number(tr.duration_ms ?? 0),
      resultPreview: String(tr.result_preview ?? ''),
    }));
  }, [message.metadata]);

  // Build the tool results lookup from persisted metadata (XML-based path).
  const toolResultsMap = useMemo(() => {
    if (!streamResult) return new Map<number, ToolResultRecord>();
    if (metaToolResults.length === 0) return new Map<number, ToolResultRecord>();

    const map = new Map<number, ToolResultRecord>();
    const consumed = new Set<number>();
    streamResult.segments.forEach((seg, segIdx) => {
      if (seg.type !== 'tool_call') return;
      for (let ri = 0; ri < metaToolResults.length; ri++) {
        if (consumed.has(ri)) continue;
        if (metaToolResults[ri].name === seg.toolCall.name) {
          map.set(segIdx, metaToolResults[ri]);
          consumed.add(ri);
          break;
        }
      }
    });
    return map;
  }, [streamResult, metaToolResults]);

  // Segment into actions vs conclusion (XML-based path).
  const actionResult = useMemo(() => {
    if (!streamResult) return null;
    return segmentActions(streamResult.segments, false);
  }, [streamResult]);

  // --- Path 2: Backend history ---
  // The backend persists the final assistant message with tool_calls: vec![],
  // so message.tool_calls is ALWAYS empty for history messages.
  // Instead, we use metadata.tool_results to reconstruct the action segment.
  const historyActionSegments = useMemo((): ContentSegment[] | null => {
    // Only use this path when XML-based parsing yielded nothing.
    if (streamResult) return null;
    // No tool results in metadata => no actions to show.
    if (metaToolResults.length === 0) return null;

    // Build synthetic tool_call segments from metadata.tool_results.
    return metaToolResults.map((tr) => ({
      type: 'tool_call' as const,
      toolCall: {
        name: tr.name,
        arguments: tr.arguments ?? '',
        startIndex: 0,
      },
    }));
  }, [streamResult, metaToolResults]);

  // Build tool results map for the history path (index by position).
  const historyToolResultsMap = useMemo(() => {
    if (!historyActionSegments) return new Map<number, ToolResultRecord>();

    const map = new Map<number, ToolResultRecord>();
    metaToolResults.forEach((tr, idx) => {
      map.set(idx, tr);
    });
    return map;
  }, [historyActionSegments, metaToolResults]);

  // Determine which rendering path to use.
  const hasXmlActions = streamResult && actionResult && actionResult.actions.length > 0;
  const hasHistoryActions = !streamResult && historyActionSegments && historyActionSegments.length > 0;

  return (
    <AssistantMessageShell message={message} isStreaming={false}>
      {hasXmlActions ? (
        /* Path 1: XML-based action segment (just-completed or accumulated content) */
        <>
          <ActionCard
            segments={actionResult!.actions}
            toolCallCount={actionResult!.toolCallCount}
            isStreaming={false}
            hasPendingToolCall={false}
            toolResultsMap={toolResultsMap}
            markdownComponents={markdownComponents}
          />
          {actionResult!.conclusion && actionResult!.conclusion.type === 'text' && (
            <div className="message-content markdown-body">
              <MarkdownSegment
                text={actionResult!.conclusion.text}
                components={markdownComponents}
              />
            </div>
          )}
        </>
      ) : hasHistoryActions ? (
        /* Path 2: Backend history -- action segment from metadata.tool_results */
        <>
          <ActionCard
            segments={historyActionSegments!}
            toolCallCount={historyActionSegments!.length}
            isStreaming={false}
            hasPendingToolCall={false}
            toolResultsMap={historyToolResultsMap}
            markdownComponents={markdownComponents}
          />
          {/* Message content is the conclusion (final LLM response) */}
          {effectiveContent.trim() && (
            <div className="message-content markdown-body">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={markdownComponents}
              >
                {effectiveContent}
              </ReactMarkdown>
            </div>
          )}
        </>
      ) : streamResult ? (
        /* Fallback: XML segments exist but no action grouping needed */
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
                : 'success';
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
        </div>
      ) : (
        /* Simple text-only message */
        <div className="message-content markdown-body">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={markdownComponents}
          >
            {effectiveContent}
          </ReactMarkdown>
        </div>
      )}
    </AssistantMessageShell>
  );
}
