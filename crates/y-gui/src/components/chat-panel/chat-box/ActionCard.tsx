/**
 * ActionCard -- collapsible container for intermediate LLM iterations.
 *
 * Groups tool calls and interleaved text from multi-iteration LLM runs
 * into a single collapsible card. Uses amber accent for visual distinction.
 *
 * - During streaming: expanded, spinner icon, "Working..." label
 * - After completion: auto-collapses, shows "N actions" count and summed duration
 */

import { useState, useEffect, useMemo } from 'react';
import { Zap, Loader } from 'lucide-react';
import type { ContentSegment } from '../../../hooks/useStreamContent';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { CollapsibleCard } from './CollapsibleCard';
import { ToolCallCard } from './ToolCallCard';
import { ThinkingCard } from './ThinkingCard';
import { MarkdownSegment } from './MessageShared';
import { extractThinkTags } from './messageUtils';
import './ActionCard.css';

interface ActionCardProps {
  /** The action segments to render (everything except the conclusion). */
  segments: ContentSegment[];
  /** Number of tool_call segments in this action group. */
  toolCallCount: number;
  /** Whether the parent message is still streaming. */
  isStreaming: boolean;
  /** Whether a tool call tag is still being buffered. */
  hasPendingToolCall: boolean;
  /** Tool results map (segment index -> result record). */
  toolResultsMap: Map<number, ToolResultRecord>;
  /** Markdown components for rendering text segments. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  markdownComponents: any;
  /** Offset to apply to segment indices when looking up tool results.
   *  Defaults to 0. */
  segmentIndexOffset?: number;
}

/** Format ms as human-readable duration. */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  return s < 60 ? `${s.toFixed(1)}s` : `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
}

const ACCENT_COLOR = '#72d077ff';

export function ActionCard({
  segments,
  toolCallCount,
  isStreaming,
  hasPendingToolCall,
  toolResultsMap,
  markdownComponents,
  segmentIndexOffset = 0,
}: ActionCardProps) {
  const [expanded, setExpanded] = useState(isStreaming);

  // Auto-collapse when streaming finishes.
  useEffect(() => {
    if (!isStreaming && expanded) {
      setExpanded(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isStreaming]);

  // Auto-expand when streaming starts.
  useEffect(() => {
    if (isStreaming) {
      setExpanded(true);
    }
  }, [isStreaming]);

  // Sum duration from all tool results for the action-group duration.
  const totalDurationMs = useMemo(() => {
    let sum = 0;
    for (const [, record] of toolResultsMap) {
      sum += record.durationMs;
    }
    return sum;
  }, [toolResultsMap]);

  const label = isStreaming
    ? 'Working...'
    : `${toolCallCount} action${toolCallCount !== 1 ? 's' : ''}`;

  const icon = isStreaming
    ? <Loader size={13} className="collapsible-card-spinner" />
    : <Zap size={13} />;

  const headerRight = !isStreaming && totalDurationMs > 0
    ? <span className="action-card-duration">{formatDuration(totalDurationMs)}</span>
    : undefined;

  return (
    <CollapsibleCard
      accentColor={ACCENT_COLOR}
      icon={icon}
      label={label}
      expanded={expanded}
      onToggle={() => setExpanded(!expanded)}
      headerRight={headerRight}
      className="action-card"
    >
      {segments.map((seg, idx) => {
        const originalIdx = idx + segmentIndexOffset;
        if (seg.type === 'text') {
          const { thinkContent, strippedContent, isThinkingIncomplete } =
            extractThinkTags(seg.text);
          return (
            <div key={`action-text-${idx}`}>
              {thinkContent && (
                <ThinkingCard
                  content={thinkContent}
                  isStreaming={isStreaming && isThinkingIncomplete}
                />
              )}
              {strippedContent.trim() && (
                <div className="markdown-body">
                  <MarkdownSegment
                    text={strippedContent}
                    components={markdownComponents}
                  />
                </div>
              )}
            </div>
          );
        }
        if (seg.type === 'tool_call') {
          const result = toolResultsMap.get(originalIdx);
          const status = result
            ? (result.success ? 'success' : 'error')
            : (isStreaming ? 'running' : 'success');
          return (
            <ToolCallCard
              key={`action-tc-${idx}`}
              toolCall={{
                id: `action-tc-${idx}`,
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
      {hasPendingToolCall && (
        <div className="tool-call-pending">
          <div className="tool-call-pending-dots">
            <span /><span /><span />
          </div>
          <span className="tool-call-pending-text">Calling tool...</span>
        </div>
      )}
    </CollapsibleCard>
  );
}
