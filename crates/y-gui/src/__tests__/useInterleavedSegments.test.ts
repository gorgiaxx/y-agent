import { describe, expect, it } from 'vitest';

import {
  appendStreamingReasoningDelta,
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
