type NavigationKey = 'ArrowDown' | 'ArrowUp';

export function nextResumeSelection(
  selectedIndex: number,
  itemCount: number,
  key: NavigationKey,
): number {
  if (itemCount === 0) return 0;
  const offset = key === 'ArrowDown' ? 1 : -1;
  return (selectedIndex + offset + itemCount) % itemCount;
}
