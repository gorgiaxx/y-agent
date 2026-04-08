/**
 * StaticBubble -- renders a completed/history assistant message.
 *
 * Renders all content and tool calls in chronological order:
 *  - XML mode: segments from processStreamContent
 *  - Native mode (metadata.tool_results): uses final_response from metadata
 *    to split content into [intermediate] [tool cards] [final answer]
 *
 * Copy button uses metadata.final_response when available (from backend),
 * falling back to the last text segment after the last tool call.
 */

import { useMemo } from 'react';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import {
  buildHistorySegments,
  extractFinalAnswer,
} from '../../../hooks/useInterleavedSegments';
import { extractXmlFinalAnswer } from '../../../hooks/useStreamContent';
import { ToolCallCard } from './ToolCallCard';
import { ThinkingCard } from './ThinkingCard';
import {
  AssistantMessageShell,
} from './MessageShared';
import { extractThinkTags } from './messageUtils';
import { useAssistantBubble } from './useAssistantBubble';
import { ThinkContentBlock } from './ThinkContentBlock';


export interface StaticBubbleProps {
  message: Message;
}


export function StaticBubble({ message }: StaticBubbleProps) {
  const effectiveContent = message.content;

  // Parse tool results from persisted metadata.
  const metaToolResults = useMemo((): ToolResultRecord[] => {
    const metaResults = message.metadata?.tool_results;
    if (!Array.isArray(metaResults)) return [];
    return (metaResults as Array<Record<string, unknown>>).map((tr) => ({
      name: String(tr.name ?? ''),
      arguments: String(tr.arguments ?? ''),
      success: Boolean(tr.success),
      durationMs: Number(tr.duration_ms ?? 0),
      resultPreview: String(tr.result_preview ?? ''),
      urlMeta: tr.url_meta != null ? JSON.stringify(tr.url_meta) : undefined,
    }));
  }, [message.metadata]);

  // Shared bubble logic: theme, parsing, toolResultsMap.
  const {
    markdownComponents,
    streamResult,
    toolResultsMap,
  } = useAssistantBubble(effectiveContent, metaToolResults);

  // Build interleaved segments for native mode history using iteration_texts.
  const historySegments = useMemo(() => {
    if (streamResult) return null; // XML mode
    const finalResponse = message.metadata?.final_response as string | undefined;
    const rawIterTexts = message.metadata?.iteration_texts;
    const iterationTexts: string[] = Array.isArray(rawIterTexts)
      ? (rawIterTexts as string[])
      : [];
    // Per-iteration reasoning from metadata.
    const rawReasonings = message.metadata?.iteration_reasonings;
    const iterationReasonings: (string | null)[] = Array.isArray(rawReasonings)
      ? (rawReasonings as (string | null)[])
      : [];
    const rawDurations = message.metadata?.iteration_reasoning_durations_ms;
    const iterationDurations: (number | null)[] = Array.isArray(rawDurations)
      ? (rawDurations as (number | null)[])
      : [];
    const rawToolCounts = message.metadata?.iteration_tool_counts;
    const iterationToolCounts: number[] = Array.isArray(rawToolCounts)
      ? (rawToolCounts as number[])
      : [];
    const finalReasoning = (message.metadata?.reasoning_content as string) ?? null;
    const finalReasoningDuration = (message.metadata?.reasoning_duration_ms as number) ?? null;
    // Need segments if there are tool results OR reasoning data.
    const hasReasoning = iterationReasonings.some(Boolean) || !!finalReasoning;
    if (metaToolResults.length === 0 && !hasReasoning) return null;
    return buildHistorySegments(
      iterationTexts,
      finalResponse,
      metaToolResults,
      iterationReasonings,
      iterationDurations,
      finalReasoning,
      finalReasoningDuration,
      iterationToolCounts,
    );
  }, [streamResult, metaToolResults, message.metadata]);

  // Copy content: the final answer only.
  const copyContent = useMemo(() => {
    const stripThink = (text: string) => extractThinkTags(text).strippedContent;

    // 1. Backend provides final_response in metadata (best source).
    const finalResponse = message.metadata?.final_response;
    if (typeof finalResponse === 'string' && finalResponse.trim()) {
      return stripThink(finalResponse);
    }

    // 2. History segments.
    if (historySegments) {
      return extractFinalAnswer(historySegments, stripThink);
    }

    // 3. XML-parsed segments.
    if (streamResult) {
      return extractXmlFinalAnswer(streamResult.segments, stripThink);
    }

    // 4. Plain text.
    return stripThink(effectiveContent);
  }, [message.metadata, historySegments, streamResult, effectiveContent]);

  return (
    <AssistantMessageShell message={message} copyContent={copyContent}>
      {streamResult ? (
        /* Path 1: XML-parsed segments */
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
                  urlMeta={result?.urlMeta}
                />
              );
            }
            return null;
          })}
        </div>
      ) : historySegments ? (
        /* Path 2: Native mode history -- interleaved via final_response split */
        <div className="message-content">
          {historySegments.map((seg, idx) => {
            if (seg.type === 'reasoning') {
              return (
                <ThinkingCard
                  key={`reason-${idx}`}
                  content={seg.content}
                  isStreaming={false}
                  durationMs={seg.durationMs}
                />
              );
            }
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
            if (seg.type === 'tool_result') {
              return (
                <ToolCallCard
                  key={`history-tc-${idx}`}
                  toolCall={{
                    id: `history-${idx}`,
                    name: seg.record.name,
                    arguments: seg.record.arguments ?? '',
                  }}
                  status={seg.record.success ? 'success' : 'error'}
                  result={seg.record.resultPreview}
                  durationMs={seg.record.durationMs}
                  urlMeta={seg.record.urlMeta}
                />
              );
            }
            return null;
          })}
        </div>
      ) : (
        /* Path 3: Plain text, no tool calls */
        <ThinkContentBlock
          content={effectiveContent}
          markdownComponents={markdownComponents}
        />
      )}
    </AssistantMessageShell>
  );
}
