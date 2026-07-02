export function formatMessageTime(timestamp: number | string | Date, now: Date = new Date()): string {
  const date = new Date(timestamp);
  const time = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  const diffMs = now.getTime() - date.getTime();
  const oneDayMs = 24 * 60 * 60 * 1000;
  const oneYearMs = 365 * oneDayMs;

  if (diffMs < oneDayMs) {
    return time;
  }

  // Always render month/day (and year when crossing the year boundary) explicitly,
  // so the output is locale-independent and matches the MM/DD contract.
  const mm = String(date.getMonth() + 1).padStart(2, '0');
  const dd = String(date.getDate()).padStart(2, '0');

  if (diffMs < oneYearMs) {
    return `${mm}/${dd} ${time}`;
  }

  const yyyy = date.getFullYear();
  return `${yyyy}/${mm}/${dd} ${time}`;
}
