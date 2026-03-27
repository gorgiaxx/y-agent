// ---------------------------------------------------------------------------
// useAssistantBubble -- shared hook for StaticBubble and StreamingBubble.
//
// Extracts the duplicated logic:
//   - Theme resolution and markdown component creation
//   - processStreamContent parsing
//   - toolResultsMap (consumed-set dedup matching)
//   - segmentActions grouping
// ---------------------------------------------------------------------------

import { useMemo } from 'react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { makeMarkdownComponents } from './MessageShared';
import { processStreamContent, synthesizeNativeStreamResult, type StreamContentResult } from '../../../hooks/useStreamContent';
import { segmentActions, type ActionSegmentResult } from '../../../hooks/useActionSegment';
import { useResolvedTheme } from '../../../hooks/useTheme';

export interface AssistantBubbleData {
  /** Resolved theme for syntax highlighting. */
  resolvedTheme: 'light' | 'dark';
  /** Memoised markdown renderer components. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  markdownComponents: any;
  /** Parsed XML segments and tool calls, or null if no tool tags found. */
  streamResult: StreamContentResult | null;
  /** Tool results keyed by segment index. */
  toolResultsMap: Map<number, ToolResultRecord>;
  /** Action/preamble/conclusion segmentation, or null if no actions. */
  actionResult: ActionSegmentResult | null;
}

/**
 * Shared preparation hook for assistant message bubbles.
 *
 * Both StaticBubble and StreamingBubble perform identical logic for:
 * 1. Theme resolution + markdown component creation
 * 2. processStreamContent XML parsing
 * 3. toolResultsMap building (consumed-set dedup)
 * 4. segmentActions grouping
 *
 * This hook consolidates that logic into one place.
 */
export function useAssistantBubble(
  content: string,
  toolResults: ToolResultRecord[],
  hasPendingToolCall?: boolean,
): AssistantBubbleData {
  // Theme + markdown components.
  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const markdownComponents = useMemo(
    () => makeMarkdownComponents(codeThemeStyle),
    [codeThemeStyle],
  );

  // Process content to extract text segments and tool call blocks.
  const streamResult = useMemo(() => {
    if (
      !content.includes('<tool_call') &&
      !content.includes('<tool_cal') &&
      !content.includes('<tool_result')
    ) {
      if (toolResults.length > 0) {
        return synthesizeNativeStreamResult(content, toolResults);
      }
      return null;
    }
    return processStreamContent(content);
  }, [content, toolResults]);

  // Build the tool results lookup by matching tool names with consumed-set dedup.
  const toolResultsMap = useMemo(() => {
    if (!streamResult) return new Map<number, ToolResultRecord>();
    if (!toolResults || toolResults.length === 0) return new Map<number, ToolResultRecord>();

    const map = new Map<number, ToolResultRecord>();
    const consumed = new Set<number>();
    streamResult.segments.forEach((seg, segIdx) => {
      if (seg.type !== 'tool_call') return;
      for (let ri = 0; ri < toolResults.length; ri++) {
        if (consumed.has(ri)) continue;
        if (toolResults[ri].name === seg.toolCall.name) {
          map.set(segIdx, toolResults[ri]);
          consumed.add(ri);
          break;
        }
      }
    });
    return map;
  }, [toolResults, streamResult]);

  // Segment into preamble, actions, and conclusion.
  const actionResult = useMemo(() => {
    if (!streamResult) return null;
    return segmentActions(
      streamResult.segments,
      hasPendingToolCall ?? streamResult.hasPendingToolCall,
    );
  }, [streamResult, hasPendingToolCall]);

  return {
    resolvedTheme,
    markdownComponents,
    streamResult,
    toolResultsMap,
    actionResult,
  };
}
