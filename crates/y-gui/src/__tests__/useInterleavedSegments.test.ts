import { describe, expect, it } from 'vitest';

import {
  appendStreamingReasoningDelta,
  buildHistorySegments,
  completeStreamingReasoningSegments,
  extractFinalAnswer,
  streamSegmentsToHistoryMetadata,
  type InterleavedSegment,
} from '../hooks/useInterleavedSegments';

/** Re-project segments through the metadata round-trip and compare ordering. */
function roundTrip(segments: InterleavedSegment[]): InterleavedSegment[] {
  const hist = streamSegmentsToHistoryMetadata(segments);
  const flatTools = segments
    .filter((s): s is Extract<InterleavedSegment, { type: 'tool_result' }> => s.type === 'tool_result')
    .map((s) => s.record);
  return buildHistorySegments(
    hist.iteration_texts,
    undefined,
    flatTools,
    hist.iteration_reasonings,
    hist.iteration_reasoning_durations_ms,
    null,
    null,
    hist.iteration_tool_counts,
    hist.injected_steers.map((s) => ({
      afterIteration: s.after_iteration,
      text: s.text,
      steerId: s.steer_id,
    })),
  );
}


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

  it('splices a steer chip between iteration tool blocks at its injection boundary', () => {
    const toolResults = [
      { name: 'Read', arguments: '', success: true, durationMs: 1, resultPreview: 'a' },
      { name: 'Grep', arguments: '', success: true, durationMs: 1, resultPreview: 'b' },
    ];
    const segments = buildHistorySegments(
      ['look\n', 'search\n'],
      'done',
      toolResults,
      [null, null],
      [null, null],
      null,
      null,
      [1, 1],
      [{ afterIteration: 1, text: 'focus on the parser', steerId: 's1' }],
    );

    expect(segments).toEqual([
      { type: 'text', text: 'look\n' },
      { type: 'tool_result', record: toolResults[0] },
      { type: 'steer', text: 'focus on the parser', steerId: 's1' },
      { type: 'text', text: 'search\n' },
      { type: 'tool_result', record: toolResults[1] },
      { type: 'text', text: 'done' },
    ]);
    // The steer chip never leaks into the copyable final answer.
    expect(extractFinalAnswer(segments, (text) => text)).toBe('done');
  });

  it('places a leading steer (afterIteration 0) before the first iteration block', () => {
    const toolResults = [
      { name: 'Read', arguments: '', success: true, durationMs: 1, resultPreview: 'a' },
    ];
    const segments = buildHistorySegments(
      ['look\n'],
      'done',
      toolResults,
      [null],
      [null],
      null,
      null,
      [1],
      [{ afterIteration: 0, text: 'wait', steerId: 's0' }],
    );

    expect(segments[0]).toEqual({ type: 'steer', text: 'wait', steerId: 's0' });
  });

  it('renders a steer chip in a turn with no tools or iterations', () => {
    const segments = buildHistorySegments(
      [],
      'final',
      [],
      [],
      [],
      null,
      null,
      [],
      [{ afterIteration: 0, text: 'reconsider', steerId: 's0' }],
    );

    expect(segments).toEqual([
      { type: 'steer', text: 'reconsider', steerId: 's0' },
      { type: 'text', text: 'final' },
    ]);
  });
});

describe('streamSegmentsToHistoryMetadata', () => {
  const toolA = { name: 'Read', arguments: '', success: true, durationMs: 1, resultPreview: 'a' };
  const toolB = { name: 'Grep', arguments: '', success: true, durationMs: 2, resultPreview: 'b' };

  it('round-trips reasoning + text + tool interleaving through buildHistorySegments', () => {
    const segments: InterleavedSegment[] = [
      { type: 'reasoning', content: 'think 1', durationMs: 100 },
      { type: 'text', text: 'I will read the file.' },
      { type: 'tool_result', record: toolA },
      { type: 'reasoning', content: 'think 2', durationMs: 200 },
      { type: 'text', text: 'Now searching.' },
      { type: 'tool_result', record: toolB },
      { type: 'text', text: 'Done.' },
    ];

    expect(roundTrip(segments)).toEqual(segments);
  });

  it('captures text that precedes a tool call so it is not dropped', () => {
    const segments: InterleavedSegment[] = [
      { type: 'text', text: 'Let me check.' },
      { type: 'tool_result', record: toolA },
    ];

    const hist = streamSegmentsToHistoryMetadata(segments);
    expect(hist.iteration_texts).toEqual(['Let me check.']);
    expect(hist.iteration_tool_counts).toEqual([1]);
    expect(roundTrip(segments)).toEqual(segments);
  });

  it('round-trips a steer chip anchored between tool blocks', () => {
    const segments: InterleavedSegment[] = [
      { type: 'text', text: 'look' },
      { type: 'tool_result', record: toolA },
      { type: 'steer', text: 'focus on the parser', steerId: 's1' },
      { type: 'text', text: 'search' },
      { type: 'tool_result', record: toolB },
    ];

    expect(roundTrip(segments)).toEqual(segments);
  });

  it('flags reasoning presence so the flat reasoning copy can be dropped', () => {
    expect(streamSegmentsToHistoryMetadata([{ type: 'text', text: 'hi' }]).hasReasoning).toBe(false);
    expect(
      streamSegmentsToHistoryMetadata([{ type: 'reasoning', content: 'r' }]).hasReasoning,
    ).toBe(true);
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

  it('only completes the live reasoning segment for the matching source', () => {
    const segments = completeStreamingReasoningSegments(
      [
        {
          type: 'reasoning',
          content: 'Phase 1 reasoning',
          isStreaming: true,
          sourceKey: 'plan-phase-executor:phase-1',
          _startTs: 1_000,
        },
        {
          type: 'reasoning',
          content: 'Phase 2 reasoning',
          isStreaming: true,
          sourceKey: 'plan-phase-executor:phase-2',
          _startTs: 1_100,
        },
      ],
      1_800,
      'plan-phase-executor:phase-1',
    );

    expect(segments).toEqual([
      {
        type: 'reasoning',
        content: 'Phase 1 reasoning',
        isStreaming: false,
        sourceKey: 'plan-phase-executor:phase-1',
        _startTs: 1_000,
        durationMs: 800,
      },
      {
        type: 'reasoning',
        content: 'Phase 2 reasoning',
        isStreaming: true,
        sourceKey: 'plan-phase-executor:phase-2',
        _startTs: 1_100,
      },
    ]);
  });
});

describe('appendStreamingReasoningDelta', () => {
  it('keeps interleaved concurrent phase reasoning in separate complete segments', () => {
    let segments = appendStreamingReasoningDelta(
      [],
      'phase 1 first chunk. ',
      'plan-phase-executor:phase-1',
      1_000,
    );
    segments = appendStreamingReasoningDelta(
      segments,
      'phase 2 only chunk.',
      'plan-phase-executor:phase-2',
      1_050,
    );
    segments = appendStreamingReasoningDelta(
      segments,
      'phase 1 second chunk.',
      'plan-phase-executor:phase-1',
      1_100,
    );

    expect(segments).toEqual([
      {
        type: 'reasoning',
        content: 'phase 1 first chunk. phase 1 second chunk.',
        isStreaming: true,
        sourceKey: 'plan-phase-executor:phase-1',
        _startTs: 1_000,
      },
      {
        type: 'reasoning',
        content: 'phase 2 only chunk.',
        isStreaming: true,
        sourceKey: 'plan-phase-executor:phase-2',
        _startTs: 1_050,
      },
    ]);
  });
});
