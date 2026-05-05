export const AUTO_SCROLL_BOTTOM_THRESHOLD_PX = 24;

interface ScrollMetrics {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}

export interface ChatScrollState {
  isAtBottom: boolean;
  shouldAutoScroll: boolean;
}

type ChatScrollEvent =
  | {
      type: 'viewport-scroll';
      metrics: ScrollMetrics;
      threshold?: number;
    }
  | {
      type: 'at-bottom-change';
      isAtBottom: boolean;
    }
  | { type: 'jump-to-bottom' };

interface FollowOutputBehaviorParams {
  shouldAutoScroll: boolean;
  isAtBottom: boolean;
}

interface FollowScrollTopParams {
  shouldAutoScroll: boolean;
  scrollHeight: number;
  clientHeight: number;
}

export const INITIAL_CHAT_SCROLL_STATE: ChatScrollState = {
  isAtBottom: true,
  shouldAutoScroll: true,
};

export function isNearBottom(
  metrics: ScrollMetrics,
  threshold = AUTO_SCROLL_BOTTOM_THRESHOLD_PX,
): boolean {
  const distanceToBottom = metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight;
  return distanceToBottom <= threshold;
}

export function reduceChatScrollState(
  _state: ChatScrollState,
  event: ChatScrollEvent,
): ChatScrollState {
  if (event.type === 'jump-to-bottom') {
    return INITIAL_CHAT_SCROLL_STATE;
  }

  if (event.type === 'at-bottom-change') {
    return event.isAtBottom
      ? INITIAL_CHAT_SCROLL_STATE
      : {
          isAtBottom: false,
          shouldAutoScroll: false,
        };
  }

  const isAtBottom = isNearBottom(event.metrics, event.threshold);
  return {
    isAtBottom,
    shouldAutoScroll: isAtBottom,
  };
}

export function shouldShowScrollToBottomButton(
  state: ChatScrollState,
  isStreaming: boolean,
): boolean {
  return isStreaming && !state.isAtBottom;
}

export function resolveFollowOutputBehavior({
  shouldAutoScroll,
  isAtBottom,
}: FollowOutputBehaviorParams): 'auto' | false {
  return shouldAutoScroll && isAtBottom ? 'auto' : false;
}

export function resolveFollowScrollTop({
  shouldAutoScroll,
  scrollHeight,
  clientHeight,
}: FollowScrollTopParams): number | null {
  if (!shouldAutoScroll) {
    return null;
  }

  return Math.max(0, scrollHeight - clientHeight);
}
