// Action segment segmentation utility.
//
// Splits a processed content segment list into three groups:
//
//   1. preamble  -- leading text segments before the first tool_call
//                   (rendered before the ActionCard)
//   2. actions   -- segments from the first tool_call up to (but not
//                   including) the trailing text conclusion
//   3. conclusion -- the trailing text segment from the last LLM
//                    iteration (rendered after the ActionCard)
//
// Pure function -- no React hooks or side-effects.

import type { ContentSegment } from './useStreamContent';

export interface ActionSegmentResult {
  /** Leading text segments before the first tool_call.  Rendered before
   *  the ActionCard (e.g. initial reasoning / <think> block). */
  preamble: ContentSegment[];
  /** Intermediate segments (tool calls + interleaved text after the
   *  first tool_call, but not the trailing conclusion). */
  actions: ContentSegment[];
  /** The trailing text segment that forms the final conclusion. Null if
   *  the last segment is a tool call (still running or no conclusion). */
  conclusion: ContentSegment | null;
  /** Count of tool_call segments within actions. */
  toolCallCount: number;
}

/**
 * Segment content into preamble, actions, and conclusion.
 *
 * Rules:
 *  - If there are no tool_call segments, returns empty actions / preamble
 *    and null conclusion (caller should render the content as-is).
 *  - Leading text segments before the first tool_call go into `preamble`.
 *  - Everything from the first tool_call up to (but not including) the
 *    trailing text segment goes into `actions`.
 *  - If the very last segment is text with non-trivial content and
 *    there is no pending tool call, it becomes the `conclusion`.
 *  - If the very last segment is a tool_call (or a pending tool call is
 *    buffering), all non-preamble segments are actions and conclusion
 *    is null.
 */
export function segmentActions(
  segments: ContentSegment[],
  hasPendingToolCall: boolean = false,
): ActionSegmentResult {
  const empty: ActionSegmentResult = {
    preamble: [],
    actions: [],
    conclusion: null,
    toolCallCount: 0,
  };

  if (!segments || segments.length === 0) return empty;

  // Count tool_call segments.
  const toolCallCount = segments.filter((s) => s.type === 'tool_call').length;
  if (toolCallCount === 0) return empty;

  // Find the first tool_call index.
  const firstToolIdx = segments.findIndex((s) => s.type === 'tool_call');

  // Preamble: all text segments before the first tool_call.
  const preamble = segments.slice(0, firstToolIdx);

  // Remaining segments (from first tool_call onward).
  const rest = segments.slice(firstToolIdx);

  const last = rest[rest.length - 1];

  // If the last segment is a text segment with non-trivial content and
  // we are NOT waiting for a pending tool call, treat it as the conclusion.
  if (
    last.type === 'text' &&
    last.text.trim().length > 0 &&
    !hasPendingToolCall
  ) {
    return {
      preamble,
      actions: rest.slice(0, -1),
      conclusion: last,
      toolCallCount,
    };
  }

  // Everything (after preamble) is actions -- last segment is a tool_call,
  // or streaming is still in progress with a pending tool call.
  return {
    preamble,
    actions: [...rest],
    conclusion: null,
    toolCallCount,
  };
}
