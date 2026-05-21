import { describe, it, expect } from 'vitest';
import { formatMessageTime } from '../utils/formatMessageTime';

describe('formatMessageTime', () => {
  const now = new Date('2026-05-21T12:00:00');

  it('shows only time for timestamps within last 24 hours', () => {
    const ts = new Date('2026-05-21T08:30:00');
    const result = formatMessageTime(ts, now);
    expect(result).not.toMatch(/\d{2}\/\d{2}/);
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });

  it('shows month/day plus time for timestamps older than 24h but same year', () => {
    const ts = new Date('2026-03-10T08:30:00');
    const result = formatMessageTime(ts, now);
    expect(result).toMatch(/03\/10/);
    expect(result).toMatch(/\d{1,2}:\d{2}/);
    expect(result).not.toMatch(/2026/);
  });

  it('shows year/month/day plus time for timestamps older than one year', () => {
    const ts = new Date('2024-03-10T08:30:00');
    const result = formatMessageTime(ts, now);
    expect(result).toMatch(/2024/);
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });
});
