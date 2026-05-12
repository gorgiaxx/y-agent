import { describe, expect, it } from 'vitest';

import {
  buildHistorySegments,
  completeStreamingReasoningSegments,
  extractFinalAnswer,
} from '../hooks/useInterleavedSegments';

describe('buildHistorySegments', () => {
  it('keeps the final response for single-turn messages that only have reasoning metadata', () => {
    const segments = buildHistorySegments(
      [],
      'Final answer',
      [],
      [],
      [],
      'step by step',
      120,
      [],
    );

    expect(segments).toEqual([
      { type: 'reasoning', content: 'step by step', durationMs: 120 },
      { type: 'text', text: 'Final answer' },
    ]);
    expect(extractFinalAnswer(segments, (text) => text)).toBe('Final answer');
  });
});

describe('completeStreamingReasoningSegments', () => {
  it('locks a live reasoning segment duration before a tool call is rendered', () => {
    const segments = completeStreamingReasoningSegments(
      [
        {
          type: 'reasoning',
          content: 'Inspecting the file before reading it.',
          isStreaming: true,
          _startTs: 1_000,
        },
      ],
      1_750,
    );

    expect(segments).toEqual([
      {
        type: 'reasoning',
        content: 'Inspecting the file before reading it.',
        isStreaming: false,
        _startTs: 1_000,
        durationMs: 750,
      },
    ]);
  });
});
