import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(async () => ({ duplicate: false })),
    listen: vi.fn(async () => () => {}),
  },
}));

import { submitAssistantFeedback } from '../lib/assistantFeedback';
import { transport } from '../lib';

describe('assistant evolution feedback', () => {
  beforeEach(() => {
    vi.mocked(transport.invoke).mockClear();
  });

  it('submits positive feedback with stable trace and feedback identifiers', async () => {
    await submitAssistantFeedback({
      traceId: '11111111-1111-4111-8111-111111111111',
      feedbackId: '22222222-2222-4222-8222-222222222222',
      rating: 'good',
    });

    expect(transport.invoke).toHaveBeenCalledWith('chat_feedback', {
      traceId: '11111111-1111-4111-8111-111111111111',
      feedbackId: '22222222-2222-4222-8222-222222222222',
      score: 1,
      comment: undefined,
    });
  });

  it('requires an actionable correction for negative feedback', async () => {
    await expect(submitAssistantFeedback({
      traceId: '11111111-1111-4111-8111-111111111111',
      feedbackId: '33333333-3333-4333-8333-333333333333',
      rating: 'bad',
      comment: '   ',
    })).rejects.toThrow('correction');
    expect(transport.invoke).not.toHaveBeenCalled();

    await submitAssistantFeedback({
      traceId: '11111111-1111-4111-8111-111111111111',
      feedbackId: '33333333-3333-4333-8333-333333333333',
      rating: 'bad',
      comment: 'The answer ignored the rollback constraint.',
    });
    expect(transport.invoke).toHaveBeenCalledWith('chat_feedback', {
      traceId: '11111111-1111-4111-8111-111111111111',
      feedbackId: '33333333-3333-4333-8333-333333333333',
      score: 0,
      comment: 'The answer ignored the rollback constraint.',
    });
  });
});
