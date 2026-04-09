// ---------------------------------------------------------------------------
// useAssistantBubble -- shared hook for StaticBubble and StreamingBubble.
//
// Extracts the duplicated logic:
//   - Theme resolution and markdown component creation
//   - processStreamContent parsing
//   - toolResultsMap (consumed-set dedup matching)
// ---------------------------------------------------------------------------

import { useMemo } from 'react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import type { ToolResultRecord } from '../../../hooks/chatStreamTypes';
import { makeMarkdownComponents } from './messageUtils';
import { processStreamContent, type StreamContentResult } from '../../../hooks/useStreamContent';
import { useResolvedTheme } from '../../../hooks/useTheme';

export interface AssistantBubbleData {
  /** Resolved theme for syntax highlighting. */
  resolvedTheme: 'light' | 'dark';
  /** Memoised markdown renderer components. */
  markdownComponents: Record<string, unknown>;
  /** Parsed XML segments and tool calls, or null if no tool tags found. */
  streamResult: StreamContentResult | null;
  /** Tool results keyed by segment index. */
  toolResultsMap: Map<number, ToolResultRecord>;
}

/**
 * Shared preparation hook for assistant message bubbles.
 *
 * Both StaticBubble and StreamingBubble perform identical logic for:
 * 1. Theme resolution + markdown component creation
 * 2. processStreamContent XML parsing
 * 3. toolResultsMap building (consumed-set dedup)
 *
 * This hook consolidates that logic into one place.
 */
export function useAssistantBubble(
  content: string,
  toolResults: ToolResultRecord[],
): AssistantBubbleData {
  // Theme + markdown components.
  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const markdownComponents = useMemo(
    () => makeMarkdownComponents(codeThemeStyle),
    [codeThemeStyle],
  );

  // Process content to extract text segments and tool call blocks.
  // Only applies when content contains XML tool_call tags (prompt-based mode).
  const streamResult = useMemo(() => {
    if (
      !content.includes('<tool_call') &&
      !content.includes('<tool_cal') &&
      !content.includes('<tool_result')
    ) {
      return null;
    }
    return processStreamContent(content);
  }, [content]);

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

  return {
    resolvedTheme,
    markdownComponents,
    streamResult,
    toolResultsMap,
  };
}
