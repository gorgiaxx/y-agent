import { describe, expect, it } from 'vitest';

import { resolveAskUserManualAction } from '../components/chat-panel/input-area/askUserDialogFlow';

describe('AskUserDialog completion flow', () => {
  it('offers a confirm path for final single-select custom text answers', () => {
    expect(resolveAskUserManualAction({
      isLastStep: true,
      isMulti: false,
      selections: ['__other__'],
    })).toBe('confirm');
  });

  it('offers a next path for non-final single-select custom text answers', () => {
    expect(resolveAskUserManualAction({
      isLastStep: false,
      isMulti: false,
      selections: ['__other__'],
    })).toBe('next');
  });

  it('keeps ordinary single-select answers on the automatic path', () => {
    expect(resolveAskUserManualAction({
      isLastStep: true,
      isMulti: false,
      selections: ['Use existing option'],
    })).toBeNull();
  });
});
