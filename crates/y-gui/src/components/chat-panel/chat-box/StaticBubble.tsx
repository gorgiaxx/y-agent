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
 *
 * Shared logic (theme, parsing, segmentation) is delegated to useAssistantBubble.
 * The ThinkContentBlock component handles the repeated think-tag extraction pattern.
 */

import { useMemo } from 'react';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { ToolCallCard } from './ToolCallCard';
import {
  AssistantMessageShell,
} from './MessageShared';
import { extractThinkTags } from './messageUtils';
import { type ContentSegment } from '../../../hooks/useStreamContent';
import { ActionCard } from './ActionCard';
import { useAssistantBubble } from './useAssistantBubble';
import { ThinkContentBlock } from './ThinkContentBlock';


export interface StaticBubbleProps {
  message: Message;
}


export function StaticBubble({ message }: StaticBubbleProps) {
  const effectiveContent = message.content;

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

  // Shared bubble logic: theme, parsing, segmentation, toolResultsMap.
  const {
    markdownComponents,
    streamResult,
    toolResultsMap,
    actionResult,
  } = useAssistantBubble(effectiveContent, metaToolResults);

  // Build synthetic action segments from metadata.tool_results (history path).
  const historyActionSegments = useMemo((): ContentSegment[] | null => {
    // Only use this path when XML-based parsing yielded nothing.
    if (streamResult) return null;
    // No tool results in metadata => no actions to show.
    if (metaToolResults.length === 0) return null;

    const segments: ContentSegment[] = [];
    metaToolResults.forEach((tr) => {
      segments.push({
        type: 'tool_call' as const,
        toolCall: {
          name: tr.name,
          arguments: tr.arguments ?? '',
          startIndex: 0,
        },
      });
    });
    return segments;
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

  // Compute the text to use for the copy button: only the final answer
  // (last LLM call's strippedContent, with think tags removed).
  const copyContent = useMemo(() => {
    if (hasXmlActions && actionResult!.conclusion?.type === 'text') {
      // XML actions path: conclusion is the final LLM answer.
      return extractThinkTags(actionResult!.conclusion.text).strippedContent;
    }
    if (hasHistoryActions) {
      // History actions path: effectiveContent is the final LLM answer.
      return extractThinkTags(effectiveContent).strippedContent;
    }
    if (streamResult) {
      // Fallback XML segments: find the last text segment.
      const textSegs = streamResult.segments.filter((s) => s.type === 'text');
      const last = textSegs[textSegs.length - 1];
      if (last?.type === 'text') {
        return extractThinkTags(last.text).strippedContent;
      }
    }
    // Plain text (no actions): strip think tags from the full content.
    return extractThinkTags(effectiveContent).strippedContent;
  }, [hasXmlActions, hasHistoryActions, actionResult, streamResult, effectiveContent]);

  return (
    <AssistantMessageShell message={message} isStreaming={false} copyContent={copyContent}>
      {hasXmlActions ? (
        /* Path 1: XML-based action segment (just-completed or accumulated content) */
        <>
          {/* Preamble: text segments before the first tool call */}
          {actionResult!.preamble.map((seg, idx) => {
            if (seg.type !== 'text') return null;
            return (
              <ThinkContentBlock
                key={`preamble-${idx}`}
                content={seg.text}
                markdownComponents={markdownComponents}
              />
            );
          })}
          <ActionCard
            segments={actionResult!.actions}
            toolCallCount={actionResult!.toolCallCount}
            isStreaming={false}
            hasPendingToolCall={false}
            toolResultsMap={toolResultsMap}
            markdownComponents={markdownComponents}
            segmentIndexOffset={actionResult!.preamble.length}
          />
          {/* Conclusion: trailing text from final LLM iteration */}
          {actionResult!.conclusion && actionResult!.conclusion.type === 'text' && (
            <ThinkContentBlock
              content={actionResult!.conclusion.text}
              markdownComponents={markdownComponents}
            />
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
            <ThinkContentBlock
              content={effectiveContent}
              markdownComponents={markdownComponents}
            />
          )}
        </>
      ) : streamResult ? (
        /* Fallback: XML segments exist but no action grouping needed */
        <div className="message-content">
          {streamResult.segments.map((seg, idx) => {
            if (seg.type === 'text') {
              return (
                <ThinkContentBlock
                  key={`text-${idx}`}
                  content={seg.text}
                  markdownComponents={markdownComponents}
                  className="markdown-body"
                />
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
        <ThinkContentBlock
          content={effectiveContent}
          markdownComponents={markdownComponents}
        />
      )}
    </AssistantMessageShell>
  );
}
