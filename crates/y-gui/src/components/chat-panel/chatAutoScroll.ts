export const AUTO_SCROLL_BOTTOM_THRESHOLD_PX = 150;

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
  | { type: 'jump-to-bottom' };

interface AutoScrollBehaviorParams {
  shouldAutoScroll: boolean;
  previousItemCount: number;
  nextItemCount: number;
  isStreaming?: boolean;
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

export function resolveAutoScrollBehavior({
  shouldAutoScroll,
  previousItemCount,
  nextItemCount,
  isStreaming = false,
}: AutoScrollBehaviorParams): 'auto' | 'smooth' | false {
  if (!shouldAutoScroll || nextItemCount <= 0) {
    return false;
  }

  if (isStreaming) {
    return 'auto';
  }

  if (previousItemCount > 0 && nextItemCount > previousItemCount) {
    return 'smooth';
  }

  return 'auto';
}
