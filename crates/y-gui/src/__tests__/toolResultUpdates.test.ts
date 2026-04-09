import { describe, expect, it } from 'vitest';
import {
  upsertToolResultRecord,
  upsertToolResultSegment,
} from '../hooks/toolResultUpdates';

const ASK_ARGS = JSON.stringify({
  questions: [
    {
      question: 'Which library?',
      options: ['React', 'Vue'],
    },
  ],
});

function makeAskUserResult(status: 'pending' | 'answered') {
  return {
    name: 'AskUser',
    arguments: ASK_ARGS,
    success: true,
    durationMs: status === 'pending' ? 12 : 128,
    resultPreview: JSON.stringify(
      status === 'pending'
        ? {
            status: 'pending',
            questions: [
              {
                question: 'Which library?',
                options: ['React', 'Vue'],
              },
            ],
          }
        : {
            status: 'answered',
            questions: [
              {
                question: 'Which library?',
                options: ['React', 'Vue'],
              },
            ],
            answers: {
              'Which library?': 'React',
            },
          },
    ),
  };
}

describe('toolResultUpdates', () => {
  it('replaces a pending AskUser result with the answered result', () => {
    const pending = makeAskUserResult('pending');
    const answered = makeAskUserResult('answered');

    const updated = upsertToolResultRecord([pending], answered);

    expect(updated.replacedIndex).toBe(0);
    expect(updated.records).toHaveLength(1);
    expect(updated.records[0]).toEqual(answered);
  });

  it('replaces the existing AskUser tool_result segment instead of appending', () => {
    const pending = makeAskUserResult('pending');
    const answered = makeAskUserResult('answered');

    const updated = upsertToolResultSegment(
      [
        { type: 'text', text: 'Need your input.' },
        { type: 'tool_result', record: pending },
      ],
      answered,
    );

    expect(updated.replacedIndex).toBe(1);
    expect(updated.segments).toHaveLength(2);
    expect(updated.segments[1]).toEqual({ type: 'tool_result', record: answered });
  });

  it('appends non-AskUser results normally', () => {
    const first = {
      name: 'Browser',
      arguments: JSON.stringify({ action: 'navigate', url: 'https://example.com' }),
      success: true,
      durationMs: 10,
      resultPreview: JSON.stringify({ status: 'ok', url: 'https://example.com' }),
    };
    const second = {
      ...first,
      resultPreview: JSON.stringify({ status: 'ok', url: 'https://example.org' }),
    };

    const updated = upsertToolResultRecord([first], second);

    expect(updated.replacedIndex).toBeNull();
    expect(updated.records).toHaveLength(2);
    expect(updated.records[1]).toEqual(second);
  });
});
