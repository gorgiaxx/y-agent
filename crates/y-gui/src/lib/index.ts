// Transport singleton -- selects Tauri or HTTP based on environment.
//
// Usage:
//   import { transport, platform } from '../lib';
//   const sessions = await transport.invoke<Session[]>('session_list');

import type { Transport } from './transport';
import type { Platform } from './platform';
import { createPlatform } from './platform';
import { TauriTransport } from './tauriTransport';
import { HttpTransport } from './httpTransport';

function detectBackend(): 'tauri' | 'http' {
  const env = import.meta.env.VITE_BACKEND as string | undefined;
  if (env === 'http') return 'http';
  if (env === 'tauri') return 'tauri';
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) return 'tauri';
  return 'http';
}

function getApiUrl(): string {
  return (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/+$/, '') ?? 'http://localhost:3000';
}

function getApiToken(): string | null {
  return (import.meta.env.VITE_API_TOKEN as string | undefined) ?? null;
}

function createTransport(): Transport {
  const backend = detectBackend();
  if (backend === 'tauri') {
    return new TauriTransport();
  }
  return new HttpTransport(getApiUrl(), getApiToken());
}

export const transport: Transport = createTransport();
export const platform: Platform = createPlatform(getApiUrl());

export type { Transport } from './transport';
export type { UnlistenFn } from './transport';
export type { Platform, OpenDialogOptions, FileFilter } from './platform';
