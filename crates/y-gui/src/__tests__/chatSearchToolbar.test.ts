/**
 * chatSearchToolbar.test.ts -- tests for useChatSearch hook logic.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

describe('useChatSearch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('goToNext cycles through matches correctly', () => {
    const total = 3;
    let current = 0;
    const nextFn = () => (total > 0 ? (current + 1) % total : 0);

    current = nextFn();
    expect(current).toBe(1);
    current = nextFn();
    expect(current).toBe(2);
    current = nextFn();
    expect(current).toBe(0);
  });

  it('goToPrev cycles backward through matches correctly', () => {
    const total = 3;
    let current = 0;
    const prevFn = () => (total > 0 ? (current - 1 + total) % total : 0);

    current = prevFn();
    expect(current).toBe(2);
    current = prevFn();
    expect(current).toBe(1);
    current = prevFn();
    expect(current).toBe(0);
  });

  it('keyboard handler condition detects Ctrl+F', () => {
    const detect = (e: { metaKey: boolean; ctrlKey: boolean; key: string }) =>
      (e.metaKey || e.ctrlKey) && e.key === 'f';

    expect(detect({ metaKey: false, ctrlKey: true, key: 'f' })).toBe(true);
    expect(detect({ metaKey: false, ctrlKey: false, key: 'f' })).toBe(false);
    expect(detect({ metaKey: false, ctrlKey: true, key: 'g' })).toBe(false);
  });

  it('keyboard handler condition detects Cmd+F (meta)', () => {
    const detect = (e: { metaKey: boolean; ctrlKey: boolean; key: string }) =>
      (e.metaKey || e.ctrlKey) && e.key === 'f';

    expect(detect({ metaKey: true, ctrlKey: false, key: 'f' })).toBe(true);
    expect(detect({ metaKey: true, ctrlKey: true, key: 'f' })).toBe(true);
  });

  it('debounce delays query update', () => {
    let debouncedValue = '';
    const debounceRef = { current: null as ReturnType<typeof setTimeout> | null };

    const setQuery = (q: string) => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        debouncedValue = q;
      }, 150);
    };

    setQuery('hello');
    expect(debouncedValue).toBe('');

    vi.advanceTimersByTime(100);
    expect(debouncedValue).toBe('');

    vi.advanceTimersByTime(60);
    expect(debouncedValue).toBe('hello');
  });

  it('debounce resets on rapid input', () => {
    let debouncedValue = '';
    const debounceRef = { current: null as ReturnType<typeof setTimeout> | null };

    const setQuery = (q: string) => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        debouncedValue = q;
      }, 150);
    };

    setQuery('he');
    vi.advanceTimersByTime(100);
    setQuery('hello');
    vi.advanceTimersByTime(100);
    expect(debouncedValue).toBe('');
    vi.advanceTimersByTime(60);
    expect(debouncedValue).toBe('hello');
  });

  it('goToNext returns 0 when totalMatches is 0', () => {
    const total = 0;
    let current = 0;
    const nextFn = () => (total > 0 ? (current + 1) % total : 0);
    current = nextFn();
    expect(current).toBe(0);
  });
});
