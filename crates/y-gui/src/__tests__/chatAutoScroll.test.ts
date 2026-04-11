import { describe, expect, it } from 'vitest';

import {
  AUTO_SCROLL_BOTTOM_THRESHOLD_PX,
  isNearBottom,
  resolveAutoScrollBehavior,
} from '../components/chat-panel/chatAutoScroll';

describe('chat auto scroll', () => {
  it('treats a viewport within the bottom threshold as near bottom', () => {
    expect(
      isNearBottom({
        scrollHeight: 1_000,
        scrollTop: 760,
        clientHeight: 120,
      }),
    ).toBe(true);
  });

  it('treats a viewport beyond the bottom threshold as not near bottom', () => {
    expect(
      isNearBottom({
        scrollHeight: 1_000,
        scrollTop: 700,
        clientHeight: 120,
      }),
    ).toBe(false);
  });

  it('uses smooth scrolling when auto-scroll stays enabled and new items are appended', () => {
    expect(
      resolveAutoScrollBehavior({
        shouldAutoScroll: true,
        previousItemCount: 3,
        nextItemCount: 4,
      }),
    ).toBe('smooth');
  });

  it('uses auto scrolling when streaming grows the current bottom item', () => {
    expect(
      resolveAutoScrollBehavior({
        shouldAutoScroll: true,
        previousItemCount: 4,
        nextItemCount: 4,
      }),
    ).toBe('auto');
  });

  it('disables scrolling when the user has left the bottom', () => {
    expect(
      resolveAutoScrollBehavior({
        shouldAutoScroll: false,
        previousItemCount: 4,
        nextItemCount: 5,
      }),
    ).toBe(false);
  });

  it('uses auto instead of smooth on the first render', () => {
    expect(
      resolveAutoScrollBehavior({
        shouldAutoScroll: true,
        previousItemCount: 0,
        nextItemCount: 2,
      }),
    ).toBe('auto');
  });

  it('exports the bottom threshold used by the panel logic', () => {
    expect(AUTO_SCROLL_BOTTOM_THRESHOLD_PX).toBe(150);
  });
});
