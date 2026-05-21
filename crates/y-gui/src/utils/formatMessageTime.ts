export function formatMessageTime(timestamp: number | string | Date, now: Date = new Date()): string {
  const date = new Date(timestamp);
  const time = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  const diffMs = now.getTime() - date.getTime();
  const oneDayMs = 24 * 60 * 60 * 1000;
  const oneYearMs = 365 * oneDayMs;

  if (diffMs < oneDayMs) {
    return time;
  }

  if (diffMs < oneYearMs) {
    const md = date.toLocaleDateString([], { month: '2-digit', day: '2-digit' });
    return `${md} ${time}`;
  }

  const ymd = date.toLocaleDateString([], { year: 'numeric', month: '2-digit', day: '2-digit' });
  return `${ymd} ${time}`;
}
