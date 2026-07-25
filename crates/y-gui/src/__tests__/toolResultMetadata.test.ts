import { describe, expect, it } from 'vitest';

import { mergeToolResultMetadata } from '../hooks/toolResultMetadata';

describe('toolResultMetadata', () => {
  it('preserves tool call ids in streamed metadata', () => {
    const merged = mergeToolResultMetadata(undefined, [
      {
        toolCallId: 'call-shell-1',
        name: 'ShellExec',
        arguments: '{"command":"true"}',
        success: true,
        durationMs: 4,
        resultPreview: 'ok',
      },
    ]);

    expect(merged?.[0]?.tool_call_id).toBe('call-shell-1');
  });

  it('deduplicates AskUser answered records when stream and backend metadata differ', () => {
    const askUserArgs = JSON.stringify({
      questions: [
        {
          question: 'Which library?',
          options: ['React', 'Vue'],
        },
      ],
    });

    const merged = mergeToolResultMetadata(
      [
        {
          name: 'AskUser',
          arguments: askUserArgs,
          success: true,
          duration_ms: 120,
          result_preview: JSON.stringify({
            answers: {
              'Which library?': 'React',
            },
          }),
          metadata: {},
        },
      ],
      [
        {
          name: 'AskUser',
          arguments: askUserArgs,
          success: true,
          durationMs: 80,
          resultPreview: JSON.stringify({
            answers: {
              'Which library?': 'React',
            },
          }),
        },
      ],
    );

    expect(merged).toHaveLength(1);
    expect(merged?.[0]).toMatchObject({
      name: 'AskUser',
      arguments: askUserArgs,
    });
  });
});
