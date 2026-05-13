/**
 * StreamingBubble -- renders a live-streaming assistant message.
 *
 * Renders all content and tool calls in chronological order:
 *  - XML mode: segments from processStreamContent (tool calls embedded in text)
 *  - Native mode: event-ordered segments from useChat (built from event
 *    arrival order, not character offsets)
 *
 * Text segments are rendered via ThinkContentBlock (handles <think> tags).
 * Tool segments are rendered as ToolCallCard.
 */

import { useMemo } from 'react';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/chatStreamTypes';
import type { InterleavedSegment } from '../../../hooks/useInterleavedSegments';
import { extractFinalAnswer } from '../../../hooks/useInterleavedSegments';
import { extractXmlFinalAnswer } from '../../../hooks/useStreamContent';
import { ToolCallCard } from './ToolCallCard';
import {
  AssistantMessageShell,
} from './MessageShared';
import { GeneratedImageGallery } from './GeneratedImageGallery';
import { extractThinkTags } from './messageUtils';
import { useAssistantBubble } from './useAssistantBubble';
import { ThinkContentBlock } from './ThinkContentBlock';
import { XmlSegmentList, NativeSegmentList } from './SegmentList';
import { extractGeneratedImages } from '../../../lib/generatedImages';


export interface StreamingBubbleProps {
  message: Message;
  /** Tool results from progress events (live streaming). */
  toolResults?: ToolResultRecord[];
  /** Event-ordered segments from useChat (text + tool_result interleaved
   *  by arrival order). Null when no native tool calls are present. */
  streamSegments?: InterleavedSegment[] | null;
}

export function StreamingBubble({ message, toolResults, streamSegments }: StreamingBubbleProps) {
  const effectiveContent = message.content;
  const generatedImages = useMemo(
    () => extractGeneratedImages(message.metadata),
    [message.metadata],
  );

  const {
    markdownComponents,
    streamResult,
    toolResultsMap,
  } = useAssistantBubble(effectiveContent, toolResults ?? []);

  // Copy content: the final answer only.
  const copyContent = useMemo(() => {
    const stripThink = (text: string) => extractThinkTags(text).strippedContent;

    // Native mode with event-ordered segments.
    if (streamSegments && streamSegments.length > 0) {
      return extractFinalAnswer(streamSegments, stripThink);
    }
    // XML-parsed segments.
    if (streamResult) {
      return extractXmlFinalAnswer(streamResult.segments, stripThink);
    }
    // Plain text, no tool calls.
    return stripThink(effectiveContent);
  }, [streamSegments, streamResult, effectiveContent]);

  return (
    <AssistantMessageShell message={message} copyContent={copyContent}>
      {streamResult ? (
        /* XML-parsed segments (prompt-based mode) */
        <div className="message-content">
          <XmlSegmentList
            segments={streamResult.segments}
            toolResultsMap={toolResultsMap}
            markdownComponents={markdownComponents}
            isStreaming
          />
          {streamResult.hasPendingToolCall && (
            <div className="tool-call-pending">
              <div className="tool-call-pending-dots">
                <span /><span /><span />
              </div>
              <span className="tool-call-pending-text">Calling tool...</span>
            </div>
          )}
        </div>
      ) : streamSegments && streamSegments.length > 0 ? (
        /* Native mode with event-ordered segments */
        <div className="message-content">
          <NativeSegmentList segments={streamSegments} markdownComponents={markdownComponents} />
        </div>
      ) : (
        /* Plain content (no tool calls at all) */
        effectiveContent.trim() ? (
          <ThinkContentBlock
            content={effectiveContent}
            markdownComponents={markdownComponents}
          />
        ) : null
      )}

      <GeneratedImageGallery images={generatedImages} />

      {/* Post-completion native tool calls from message.tool_calls (no results). */}
      {!streamResult && (!streamSegments || streamSegments.length === 0) && message.tool_calls.length > 0 && (
        <div className="message-tool-calls">
          {message.tool_calls.map((tc) => (
            <ToolCallCard key={tc.id} toolCall={tc} />
          ))}
        </div>
      )}
    </AssistantMessageShell>
  );
}
