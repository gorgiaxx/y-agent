// Action segment segmentation utility.
//
// Splits a processed content segment list into "actions" (intermediate
// tool calls + interleaved text) and a trailing "conclusion" (final text
// segment from the last LLM iteration).
//
// Pure function -- no React hooks or side-effects.

import type { ContentSegment } from './useStreamContent';

export interface ActionSegmentResult {
  /** Intermediate segments (tool calls + text before the conclusion). */
  actions: ContentSegment[];
  /** The trailing text segment that forms the final conclusion. Null if
   *  the last segment is a tool call (still running or no conclusion). */
  conclusion: ContentSegment | null;
  /** Count of tool_call segments within actions. */
  toolCallCount: number;
}

/**
 * Segment content into actions and conclusion.
 *
 * Rules:
 *  - If there are no tool_call segments, returns empty actions and null
 *    conclusion (caller should render the content as-is).
 *  - Otherwise, everything up to (but not including) the trailing text
 *    segment is "actions", and the trailing text is "conclusion".
 *  - If the very last segment is a tool_call, all segments are actions
 *    and conclusion is null (the tool is still pending).
 *  - During streaming with `hasPendingToolCall`, the conclusion may not
 *    exist yet.
 */
export function segmentActions(
  segments: ContentSegment[],
  hasPendingToolCall: boolean = false,
): ActionSegmentResult {
  const empty: ActionSegmentResult = { actions: [], conclusion: null, toolCallCount: 0 };

  if (!segments || segments.length === 0) return empty;

  // Count tool_call segments.
  const toolCallCount = segments.filter((s) => s.type === 'tool_call').length;
  if (toolCallCount === 0) return empty;

  const last = segments[segments.length - 1];

  // If the last segment is a text segment with non-trivial content and
  // we are NOT waiting for a pending tool call, treat it as the conclusion.
  if (
    last.type === 'text' &&
    last.text.trim().length > 0 &&
    !hasPendingToolCall
  ) {
    return {
      actions: segments.slice(0, -1),
      conclusion: last,
      toolCallCount,
    };
  }

  // Everything is actions (last segment is a tool_call, or streaming
  // is still in progress with a pending tool call).
  return {
    actions: [...segments],
    conclusion: null,
    toolCallCount,
  };
}
