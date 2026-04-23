import type { InterleavedSegment } from './useInterleavedSegments';
import type { ToolResultRecord } from './chatStreamTypes';

export const MAX_STREAM_SEGMENTS = 300;
export const STREAM_TRIM_TARGET = 200;

export const MAX_TOOL_RESULTS = 300;
export const TOOL_RESULTS_TRIM_TARGET = 200;

export function capSegments(segments: InterleavedSegment[]): InterleavedSegment[] {
  if (segments.length <= MAX_STREAM_SEGMENTS) return segments;
  return segments.slice(segments.length - STREAM_TRIM_TARGET);
}

export function capToolResults(records: ToolResultRecord[]): ToolResultRecord[] {
  if (records.length <= MAX_TOOL_RESULTS) return records;
  return records.slice(records.length - TOOL_RESULTS_TRIM_TARGET);
}
