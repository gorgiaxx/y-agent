/**
 * Segment types for interleaved content + tool call + reasoning rendering.
 *
 * Used by both StreamingBubble (event-ordered segments from useChat)
 * and StaticBubble (history segments built from iteration_texts).
 */

import type { ToolResultRecord } from './chatStreamTypes';

/** A display segment: text, tool result card, or reasoning card. */
export type InterleavedSegment =
  | { type: 'text'; text: string }
  | { type: 'tool_result'; record: ToolResultRecord }
  | { type: 'reasoning'; content: string; durationMs?: number; isStreaming?: boolean;
      /** Client-side timestamp (ms) when this reasoning segment started streaming.
       *  Used to compute durationMs when the segment completes. Not rendered. */
      _startTs?: number };

/**
 * Build interleaved segments for a history/persisted message.
 *
 * Uses `iterationTexts` (per-iteration content from metadata) and
 * `iterationReasonings` (per-iteration reasoning) to interleave
 * reasoning, text, and tool cards in the correct order.
 *
 * Layout per iteration:
 *   [reasoning_i] [text_i] [tools_i...]
 * Followed by:
 *   [final_reasoning] [final_response]
 *
 * This approach is robust because each piece of data is stored separately --
 * no character offsets or string splitting needed.
 */
export function buildHistorySegments(
  iterationTexts: string[],
  finalResponse: string | undefined,
  toolResults: ToolResultRecord[],
  iterationReasonings?: (string | null)[],
  iterationReasoningDurations?: (number | null)[],
  finalReasoning?: string | null,
  finalReasoningDuration?: number | null,
  iterationToolCounts?: number[],
): InterleavedSegment[] {
  if (!toolResults.length && !iterationTexts.length) {
    const segments: InterleavedSegment[] = [];
    if (finalReasoning) {
      segments.push({
        type: 'reasoning',
        content: finalReasoning,
        durationMs: finalReasoningDuration ?? undefined,
      });
    }
    if (finalResponse && finalResponse.trim()) {
      segments.push({ type: 'text', text: finalResponse });
    }
    return segments;
  }

  const segments: InterleavedSegment[] = [];
  let toolIdx = 0;

  // Each iteration: [reasoning] [text] [tool_results...]
  for (let i = 0; i < iterationTexts.length; i++) {
    const reasoning = iterationReasonings?.[i];
    if (reasoning) {
      segments.push({
        type: 'reasoning',
        content: reasoning,
        durationMs: iterationReasoningDurations?.[i] ?? undefined,
      });
    }
    const text = iterationTexts[i];
    if (text.trim()) {
      segments.push({ type: 'text', text });
    }
    // Distribute tool results using per-iteration counts when available.
    const count = iterationToolCounts?.[i] ?? 0;
    for (let t = 0; t < count && toolIdx < toolResults.length; t++) {
      segments.push({ type: 'tool_result', record: toolResults[toolIdx++] });
    }
  }

  // Any remaining tool results (fallback when counts are unavailable,
  // e.g. messages persisted before iteration_tool_counts was added).
  while (toolIdx < toolResults.length) {
    segments.push({ type: 'tool_result', record: toolResults[toolIdx++] });
  }

  // Final response reasoning + text.
  if (finalReasoning) {
    segments.push({
      type: 'reasoning',
      content: finalReasoning,
      durationMs: finalReasoningDuration ?? undefined,
    });
  }

  if (finalResponse && finalResponse.trim()) {
    segments.push({ type: 'text', text: finalResponse });
  }

  return segments;
}

/**
 * Extract the final answer text from interleaved segments.
 *
 * The final answer is the last text segment after the last tool_result.
 * Used by the copy button.
 */
export function extractFinalAnswer(
  segments: InterleavedSegment[],
  stripThinkFn: (text: string) => string,
): string {
  let lastToolIdx = -1;
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].type === 'tool_result') {
      lastToolIdx = i;
      break;
    }
  }

  if (lastToolIdx >= 0) {
    const afterTool = segments.slice(lastToolIdx + 1)
      .filter((s): s is { type: 'text'; text: string } => s.type === 'text')
      .map((s) => s.text)
      .join('');
    if (afterTool.trim()) return stripThinkFn(afterTool);
    return '';
  }

  const allText = segments
    .filter((s): s is { type: 'text'; text: string } => s.type === 'text')
    .map((s) => s.text)
    .join('');
  return stripThinkFn(allText);
}
