import type { BackgroundTaskLogEntry } from '../../hooks/useBackgroundTasks';

export function outputContent(logs: BackgroundTaskLogEntry[]): string {
  return logs
    .map((entry) => {
      if (entry.stream === 'stderr') {
        return `\u001b[31m${entry.content}\u001b[0m`;
      }
      return entry.content;
    })
    .join('');
}
