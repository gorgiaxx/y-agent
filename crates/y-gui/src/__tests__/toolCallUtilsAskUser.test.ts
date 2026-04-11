import { describe, expect, it } from 'vitest';

import { extractAskUserMeta } from '../components/chat-panel/chat-box/toolCallUtils';

describe('toolCallUtils AskUser metadata', () => {
  it('falls back to tool arguments when the answered result omits questions', () => {
    const meta = extractAskUserMeta(
      JSON.stringify({
        questions: [
          {
            question: 'Which library?',
            options: ['React', 'Vue'],
          },
        ],
      }),
      JSON.stringify({
        answers: {
          'Which library?': 'React',
        },
      }),
    );

    expect(meta).toEqual({
      questions: [
        {
          question: 'Which library?',
          options: ['React', 'Vue'],
        },
      ],
      status: 'answered',
    });
  });
});
