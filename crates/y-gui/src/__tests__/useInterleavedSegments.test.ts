import { describe, expect, it } from 'vitest';

import { buildHistorySegments, extractFinalAnswer } from '../hooks/useInterleavedSegments';

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
