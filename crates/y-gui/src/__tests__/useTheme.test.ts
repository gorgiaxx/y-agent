import { describe, expect, it, vi } from 'vitest';

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(),
  },
}));

import {
  resolveNativeThemePreference,
  subscribeToSystemThemeChanges,
  type NativeThemePreference,
  type SystemThemeQueryList,
} from '../hooks/useTheme';

function createModernQueryList(): {
  queryList: SystemThemeQueryList;
  emitChange: (matches: boolean) => void;
  addEventListener: ReturnType<typeof vi.fn>;
  removeEventListener: ReturnType<typeof vi.fn>;
} {
  const listeners = new Set<(event: { matches: boolean }) => void>();
  const addEventListener = vi.fn((event: string, listener: (event: { matches: boolean }) => void) => {
    if (event === 'change') listeners.add(listener);
  });
  const removeEventListener = vi.fn(
    (event: string, listener: (event: { matches: boolean }) => void) => {
      if (event === 'change') listeners.delete(listener);
    },
  );

  return {
    queryList: {
      matches: true,
      addEventListener,
      removeEventListener,
    },
    emitChange(matches: boolean) {
      listeners.forEach((listener) => listener({ matches }));
    },
    addEventListener,
    removeEventListener,
  };
}

function createLegacyQueryList(): {
  queryList: SystemThemeQueryList;
  emitChange: (matches: boolean) => void;
  addListener: ReturnType<typeof vi.fn>;
  removeListener: ReturnType<typeof vi.fn>;
} {
  const listeners = new Set<(event: { matches: boolean }) => void>();
  const addListener = vi.fn((listener: (event: { matches: boolean }) => void) => {
    listeners.add(listener);
  });
  const removeListener = vi.fn((listener: (event: { matches: boolean }) => void) => {
    listeners.delete(listener);
  });

  return {
    queryList: {
      matches: true,
      addListener,
      removeListener,
    },
    emitChange(matches: boolean) {
      listeners.forEach((listener) => listener({ matches }));
    },
    addListener,
    removeListener,
  };
}

describe('resolveNativeThemePreference', () => {
  it('keeps explicit themes pinned to the requested native mode', () => {
    expect(resolveNativeThemePreference('dark', 'dark')).toBe('dark');
    expect(resolveNativeThemePreference('light', 'light')).toBe('light');
  });

  it('clears the native override when the GUI follows the system theme', () => {
    const nativeTheme = resolveNativeThemePreference('system', 'dark');

    expect(nativeTheme satisfies NativeThemePreference).toBe('system');
  });
});

describe('subscribeToSystemThemeChanges', () => {
  it('subscribes with the modern MediaQueryList event API', () => {
    const { queryList, emitChange, addEventListener, removeEventListener } = createModernQueryList();
    const onThemeChange = vi.fn();

    const unsubscribe = subscribeToSystemThemeChanges(queryList, onThemeChange);
    emitChange(false);
    unsubscribe();

    expect(addEventListener).toHaveBeenCalledOnce();
    expect(onThemeChange).toHaveBeenCalledWith('light');
    expect(removeEventListener).toHaveBeenCalledOnce();
  });

  it('falls back to the legacy MediaQueryList listener API', () => {
    const { queryList, emitChange, addListener, removeListener } = createLegacyQueryList();
    const onThemeChange = vi.fn();

    const unsubscribe = subscribeToSystemThemeChanges(queryList, onThemeChange);
    emitChange(false);
    unsubscribe();

    expect(addListener).toHaveBeenCalledOnce();
    expect(onThemeChange).toHaveBeenCalledWith('light');
    expect(removeListener).toHaveBeenCalledOnce();
  });

  it('returns a no-op unsubscribe when query list listeners are unavailable', () => {
    const queryList: SystemThemeQueryList = { matches: true };
    const onThemeChange = vi.fn();

    const unsubscribe = subscribeToSystemThemeChanges(queryList, onThemeChange);

    expect(() => unsubscribe()).not.toThrow();
    expect(onThemeChange).not.toHaveBeenCalled();
  });
});
