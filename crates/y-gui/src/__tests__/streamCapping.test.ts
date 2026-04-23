import { describe, it, expect } from 'vitest';
import {
  capSegments,
  capToolResults,
  MAX_STREAM_SEGMENTS,
  STREAM_TRIM_TARGET,
  MAX_TOOL_RESULTS,
  TOOL_RESULTS_TRIM_TARGET,
} from '../hooks/streamCapping';
import type { InterleavedSegment } from '../hooks/useInterleavedSegments';
import type { ToolResultRecord } from '../hooks/chatStreamTypes';

function makeToolSegment(index: number): InterleavedSegment {
  return {
    type: 'tool_result',
    record: {
      name: `tool_${index}`,
      success: true,
      durationMs: 100,
      resultPreview: `result_${index}`,
    },
  };
}

function makeToolRecord(index: number): ToolResultRecord {
  return {
    name: `tool_${index}`,
    success: true,
    durationMs: 100,
    resultPreview: `result_${index}`,
  };
}

describe('capSegments', () => {
  it('returns input unchanged when under limit', () => {
    const segs: InterleavedSegment[] = [
      { type: 'text', text: 'hello' },
      makeToolSegment(0),
    ];
    const result = capSegments(segs);
    expect(result).toBe(segs);
  });

  it('returns input unchanged at exactly the limit', () => {
    const segs = Array.from({ length: MAX_STREAM_SEGMENTS }, (_, i) => makeToolSegment(i));
    const result = capSegments(segs);
    expect(result).toBe(segs);
  });

  it('trims from front to STREAM_TRIM_TARGET when one over limit', () => {
    const segs = Array.from({ length: MAX_STREAM_SEGMENTS + 1 }, (_, i) => makeToolSegment(i));
    const result = capSegments(segs);
    expect(result.length).toBe(STREAM_TRIM_TARGET);
    expect(result[0]).toEqual(makeToolSegment(MAX_STREAM_SEGMENTS + 1 - STREAM_TRIM_TARGET));
    expect(result[result.length - 1]).toEqual(makeToolSegment(MAX_STREAM_SEGMENTS));
  });

  it('trims from front preserving most recent entries', () => {
    const total = MAX_STREAM_SEGMENTS + 50;
    const segs = Array.from({ length: total }, (_, i) => makeToolSegment(i));
    const result = capSegments(segs);
    expect(result.length).toBe(STREAM_TRIM_TARGET);
    expect(result[result.length - 1]).toEqual(makeToolSegment(total - 1));
  });

  it('preserves mixed segment types after trimming', () => {
    const total = MAX_STREAM_SEGMENTS + 10;
    const segs: InterleavedSegment[] = Array.from({ length: total }, (_, i) => {
      if (i % 3 === 0) return { type: 'text', text: `text_${i}` };
      if (i % 3 === 1) return { type: 'reasoning', content: `reason_${i}` };
      return makeToolSegment(i);
    });
    const result = capSegments(segs);
    expect(result.length).toBe(STREAM_TRIM_TARGET);
    expect(result[result.length - 1]).toEqual(segs[total - 1]);
  });
});

describe('capToolResults', () => {
  it('returns input unchanged when under limit', () => {
    const records = [makeToolRecord(0), makeToolRecord(1)];
    const result = capToolResults(records);
    expect(result).toBe(records);
  });

  it('returns input unchanged at exactly the limit', () => {
    const records = Array.from({ length: MAX_TOOL_RESULTS }, (_, i) => makeToolRecord(i));
    const result = capToolResults(records);
    expect(result).toBe(records);
  });

  it('trims from front to TOOL_RESULTS_TRIM_TARGET when one over limit', () => {
    const records = Array.from({ length: MAX_TOOL_RESULTS + 1 }, (_, i) => makeToolRecord(i));
    const result = capToolResults(records);
    expect(result.length).toBe(TOOL_RESULTS_TRIM_TARGET);
    expect(result[0]).toEqual(makeToolRecord(MAX_TOOL_RESULTS + 1 - TOOL_RESULTS_TRIM_TARGET));
    expect(result[result.length - 1]).toEqual(makeToolRecord(MAX_TOOL_RESULTS));
  });

  it('trims from front preserving most recent records', () => {
    const total = MAX_TOOL_RESULTS + 50;
    const records = Array.from({ length: total }, (_, i) => makeToolRecord(i));
    const result = capToolResults(records);
    expect(result.length).toBe(TOOL_RESULTS_TRIM_TARGET);
    expect(result[result.length - 1]).toEqual(makeToolRecord(total - 1));
  });
});
