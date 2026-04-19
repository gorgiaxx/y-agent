// Theme hook -- resolves 'system' preference and exposes the effective theme
// ('dark' | 'light') to the component tree via React context.

import { createContext, useContext, useEffect, useState, useMemo } from 'react';
import { transport } from '../lib';
import type { GuiConfig } from '../types';

export type ResolvedTheme = 'dark' | 'light';
export type NativeThemePreference = GuiConfig['theme'];

interface ThemeChangeEvent {
  matches: boolean;
}

interface ThemeChangeListener {
  (event: ThemeChangeEvent): void;
}

export interface SystemThemeQueryList {
  matches: boolean;
  addEventListener?: (event: 'change', listener: ThemeChangeListener) => void;
  removeEventListener?: (event: 'change', listener: ThemeChangeListener) => void;
  addListener?: (listener: ThemeChangeListener) => void;
  removeListener?: (listener: ThemeChangeListener) => void;
}

interface ThemeContextValue {
  resolvedTheme: ResolvedTheme;
}

const ThemeContext = createContext<ThemeContextValue>({ resolvedTheme: 'dark' });

function resolveThemeFromMatch(matches: boolean): ResolvedTheme {
  return matches ? 'dark' : 'light';
}

export function resolveNativeThemePreference(
  preference: GuiConfig['theme'],
  resolvedTheme: ResolvedTheme,
): NativeThemePreference {
  return preference === 'system' ? 'system' : resolvedTheme;
}

export function subscribeToSystemThemeChanges(
  queryList: SystemThemeQueryList | null | undefined,
  onThemeChange: (theme: ResolvedTheme) => void,
): () => void {
  if (!queryList) return () => {};

  const handler: ThemeChangeListener = (event) => {
    onThemeChange(resolveThemeFromMatch(event.matches));
  };

  if (queryList.addEventListener && queryList.removeEventListener) {
    queryList.addEventListener('change', handler);
    return () => queryList.removeEventListener?.('change', handler);
  }

  if (queryList.addListener && queryList.removeListener) {
    queryList.addListener(handler);
    return () => queryList.removeListener?.(handler);
  }

  return () => {};
}

/**
 * Detect the OS-level colour scheme preference.
 * Falls back to 'dark' when matchMedia is unavailable.
 */
function getSystemTheme(): ResolvedTheme {
  if (typeof window === 'undefined' || !window.matchMedia) return 'dark';
  return resolveThemeFromMatch(window.matchMedia('(prefers-color-scheme: dark)').matches);
}

/**
 * useThemeProvider -- call this ONCE at the top level (App) to:
 *   1. Resolve 'system' to 'dark' | 'light' via matchMedia.
 *   2. Apply the effective theme to document.documentElement[data-theme].
 *   3. Return a context value to feed into ThemeContext.Provider.
 */
export function useThemeProvider(preference: GuiConfig['theme']): ThemeContextValue {
  const [systemTheme, setSystemTheme] = useState<ResolvedTheme>(getSystemTheme);

  // Listen for OS-level theme changes.
  useEffect(() => {
    const queryList = window.matchMedia?.('(prefers-color-scheme: dark)') as
      | SystemThemeQueryList
      | undefined;
    if (!queryList) return;

    return subscribeToSystemThemeChanges(queryList, setSystemTheme);
  }, []);

  const resolvedTheme: ResolvedTheme =
    preference === 'system' ? systemTheme : preference;
  const nativeThemePreference = resolveNativeThemePreference(preference, resolvedTheme);

  // Apply data-theme attribute on the root element and sync the native
  // window theme so macOS vibrancy material matches the app's mode.
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolvedTheme);
    transport.invoke('window_set_theme', { theme: nativeThemePreference }).catch(() => {});
  }, [nativeThemePreference, resolvedTheme]);

  return useMemo(() => ({ resolvedTheme }), [resolvedTheme]);
}

/**
 * useResolvedTheme -- consume the current resolved theme from any component.
 */
export function useResolvedTheme(): ResolvedTheme {
  return useContext(ThemeContext).resolvedTheme;
}

export { ThemeContext };
