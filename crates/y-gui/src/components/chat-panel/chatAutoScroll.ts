export const AUTO_SCROLL_BOTTOM_THRESHOLD_PX = 150;

interface ScrollMetrics {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}

interface AutoScrollBehaviorParams {
  shouldAutoScroll: boolean;
  previousItemCount: number;
  nextItemCount: number;
}

export function isNearBottom(
  metrics: ScrollMetrics,
  threshold = AUTO_SCROLL_BOTTOM_THRESHOLD_PX,
): boolean {
  const distanceToBottom = metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight;
  return distanceToBottom <= threshold;
}

export function resolveAutoScrollBehavior({
  shouldAutoScroll,
  previousItemCount,
  nextItemCount,
}: AutoScrollBehaviorParams): 'auto' | 'smooth' | false {
  if (!shouldAutoScroll || nextItemCount <= 0) {
    return false;
  }

  if (previousItemCount > 0 && nextItemCount > previousItemCount) {
    return 'smooth';
  }

  return 'auto';
}
