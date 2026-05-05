import { describe, expect, it } from 'vitest';

import {
  AUTO_SCROLL_BOTTOM_THRESHOLD_PX,
  INITIAL_CHAT_SCROLL_STATE,
  isNearBottom,
  reduceChatScrollState,
  resolveFollowOutputBehavior,
  resolveFollowScrollTop,
  shouldShowScrollToBottomButton,
} from '../components/chat-panel/chatAutoScroll';

describe('chat auto scroll', () => {
  it('treats a viewport within the bottom threshold as near bottom', () => {
    expect(
      isNearBottom({
        scrollHeight: 1_000,
        scrollTop: 860,
        clientHeight: 120,
      }),
    ).toBe(true);
  });

  it('treats a viewport beyond the bottom threshold as not near bottom', () => {
    expect(
      isNearBottom({
        scrollHeight: 1_000,
        scrollTop: 850,
        clientHeight: 120,
      }),
    ).toBe(false);
  });

  it('uses immediate follow output when the viewport is still pinned to bottom', () => {
    expect(
      resolveFollowOutputBehavior({
        shouldAutoScroll: true,
        isAtBottom: true,
      }),
    ).toBe('auto');
  });

  it('stops follow output as soon as the viewport leaves bottom', () => {
    expect(
      resolveFollowOutputBehavior({
        shouldAutoScroll: true,
        isAtBottom: false,
      }),
    ).toBe(false);
  });

  it('keeps follow output disabled while the user is reviewing history', () => {
    expect(
      resolveFollowOutputBehavior({
        shouldAutoScroll: false,
        isAtBottom: true,
      }),
    ).toBe(false);
  });

  it('does not force a scrollTop change while the user is reviewing history', () => {
    expect(
      resolveFollowScrollTop({
        shouldAutoScroll: false,
        scrollHeight: 3_000,
        clientHeight: 600,
      }),
    ).toBeNull();
  });

  it('pins the native scroller to the bottom only while follow mode is enabled', () => {
    expect(
      resolveFollowScrollTop({
        shouldAutoScroll: true,
        scrollHeight: 3_000,
        clientHeight: 600,
      }),
    ).toBe(2_400);
  });

  it('exports the bottom threshold used by the panel logic', () => {
    expect(AUTO_SCROLL_BOTTOM_THRESHOLD_PX).toBe(24);
  });

  it('disables follow mode when the user scrolls away from the bottom', () => {
    const next = reduceChatScrollState(INITIAL_CHAT_SCROLL_STATE, {
      type: 'viewport-scroll',
      metrics: {
        scrollHeight: 1_000,
        scrollTop: 600,
        clientHeight: 120,
      },
    });

    expect(next).toEqual({
      isAtBottom: false,
      shouldAutoScroll: false,
    });
  });

  it('re-enables follow mode when the user scrolls back to the bottom', () => {
    const next = reduceChatScrollState(
      {
        isAtBottom: false,
        shouldAutoScroll: false,
      },
      {
        type: 'viewport-scroll',
        metrics: {
          scrollHeight: 1_000,
          scrollTop: 860,
          clientHeight: 120,
        },
      },
    );

    expect(next).toEqual({
      isAtBottom: true,
      shouldAutoScroll: true,
    });
  });

  it('disables follow mode when Virtuoso reports that the user left bottom', () => {
    const next = reduceChatScrollState(INITIAL_CHAT_SCROLL_STATE, {
      type: 'at-bottom-change',
      isAtBottom: false,
    });

    expect(next).toEqual({
      isAtBottom: false,
      shouldAutoScroll: false,
    });
  });

  it('shows the jump-to-bottom button only while streaming away from bottom', () => {
    expect(
      shouldShowScrollToBottomButton(
        {
          isAtBottom: false,
          shouldAutoScroll: false,
        },
        true,
      ),
    ).toBe(true);
    expect(
      shouldShowScrollToBottomButton(
        {
          isAtBottom: true,
          shouldAutoScroll: true,
        },
        true,
      ),
    ).toBe(false);
    expect(
      shouldShowScrollToBottomButton(
        {
          isAtBottom: false,
          shouldAutoScroll: false,
        },
        false,
      ),
    ).toBe(false);
  });

  it('restores bottom follow mode after the user clicks jump to bottom', () => {
    const next = reduceChatScrollState(
      {
        isAtBottom: false,
        shouldAutoScroll: false,
      },
      { type: 'jump-to-bottom' },
    );

    expect(next).toEqual({
      isAtBottom: true,
      shouldAutoScroll: true,
    });
  });
});
