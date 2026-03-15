// Theme hook -- resolves 'system' preference and exposes the effective theme
// ('dark' | 'light') to the component tree via React context.

import { createContext, useContext, useEffect, useState, useMemo } from 'react';
import type { GuiConfig } from '../types';

export type ResolvedTheme = 'dark' | 'light';

interface ThemeContextValue {
  resolvedTheme: ResolvedTheme;
}

const ThemeContext = createContext<ThemeContextValue>({ resolvedTheme: 'dark' });

/**
 * Detect the OS-level colour scheme preference.
 * Falls back to 'dark' when matchMedia is unavailable.
 */
function getSystemTheme(): ResolvedTheme {
  if (typeof window === 'undefined' || !window.matchMedia) return 'dark';
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
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
    const mql = window.matchMedia?.('(prefers-color-scheme: dark)');
    if (!mql) return;

    const handler = (e: MediaQueryListEvent) => {
      setSystemTheme(e.matches ? 'dark' : 'light');
    };
    mql.addEventListener('change', handler);
    return () => mql.removeEventListener('change', handler);
  }, []);

  const resolvedTheme: ResolvedTheme =
    preference === 'system' ? systemTheme : preference;

  // Apply data-theme attribute on the root element.
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolvedTheme);
  }, [resolvedTheme]);

  return useMemo(() => ({ resolvedTheme }), [resolvedTheme]);
}

/**
 * useResolvedTheme -- consume the current resolved theme from any component.
 */
export function useResolvedTheme(): ResolvedTheme {
  return useContext(ThemeContext).resolvedTheme;
}

export { ThemeContext };
