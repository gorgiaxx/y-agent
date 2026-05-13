import { describe, expect, it } from 'vitest';

import {
  clearSessionInteractionByPredicate,
  getSessionInteraction,
  setSessionInteraction,
} from '../utils/sessionInteractionState';

describe('sessionInteractionState', () => {
  it('keeps AskUser interactions scoped to their originating session', () => {
    const state = setSessionInteraction({}, 'session-a', {
      interactionId: 'ask-a',
      questions: [
        {
          question: 'Choose A',
          options: ['A1', 'A2'],
        },
      ],
    });

    expect(getSessionInteraction(state, 'session-a')).toEqual({
      interactionId: 'ask-a',
      questions: [
        {
          question: 'Choose A',
          options: ['A1', 'A2'],
        },
      ],
    });
    expect(getSessionInteraction(state, 'session-b')).toBeNull();
    expect(getSessionInteraction(state, null)).toBeNull();
  });

  it('clears only the answered session interaction', () => {
    const state = setSessionInteraction(
      setSessionInteraction({}, 'session-a', {
        interactionId: 'ask-a',
        questions: [],
      }),
      'session-b',
      {
        interactionId: 'ask-b',
        questions: [],
      },
    );

    const next = clearSessionInteractionByPredicate(
      state,
      (interaction) => interaction.interactionId === 'ask-a',
    );

    expect(getSessionInteraction(next, 'session-a')).toBeNull();
    expect(getSessionInteraction(next, 'session-b')).toEqual({
      interactionId: 'ask-b',
      questions: [],
    });
  });
});
