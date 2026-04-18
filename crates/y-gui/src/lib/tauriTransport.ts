// Tauri transport implementation.
//
// Thin wrapper around @tauri-apps/api that conforms to the Transport interface.
// Used when VITE_BACKEND=tauri (default).

import type { Transport, UnlistenFn } from './transport';

export class TauriTransport implements Transport {
  async invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(command, args);
  }

  async listen<T = unknown>(
    event: string,
    callback: (event: { payload: T }) => void,
  ): Promise<UnlistenFn> {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<T>(event, callback);
  }
}
