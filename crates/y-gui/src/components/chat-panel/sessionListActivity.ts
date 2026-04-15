export function formatSessionRelativeTime(updatedAt: string, isStreaming: boolean): string {
  if (isStreaming) return 'now';

  const diffMs = Date.now() - new Date(updatedAt).getTime();
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;

  const days = Math.floor(hours / 24);
  return `${days}d`;
}
