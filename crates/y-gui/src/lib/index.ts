// Transport singleton -- selects Tauri or HTTP based on environment.
//
// Usage:
//   import { transport, platform } from '../lib';
//   const sessions = await transport.invoke<Session[]>('session_list');

import type { Transport, ConnectionStatus } from './transport';
import { platform, getApiUrl, isTauriEnvironment } from './platform';
import { TauriTransport } from './tauriTransport';
import { HttpTransport } from './httpTransport';

function detectBackend(): 'tauri' | 'http' {
  const env = import.meta.env.VITE_BACKEND as string | undefined;
  if (env === 'http') return 'http';
  if (env === 'tauri') return 'tauri';
  if (isTauriEnvironment()) return 'tauri';
  return 'http';
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
export { platform };
export { logger, createLogger } from './logger';
export type { Logger, LogLevel, LogSink } from './logger';

export function getConnectionStatus(): ConnectionStatus {
  return transport.connectionStatus ?? 'connected';
}

export function onConnectionStatusChange(cb: (status: ConnectionStatus) => void): () => void {
  return transport.onConnectionStatusChange?.(cb) ?? (() => {});
}

export type { Transport } from './transport';
export type { UnlistenFn } from './transport';
export type { Platform, OpenDialogOptions, FileFilter } from './platform';
export type { ConnectionStatus } from './transport';
