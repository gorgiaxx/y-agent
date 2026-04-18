// Transport abstraction layer.
//
// Provides a unified interface for frontend code to call backend commands
// and listen to events, regardless of whether the backend is Tauri IPC
// or a remote HTTP+SSE server.

export type UnlistenFn = () => void;

export interface Transport {
  invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T = unknown>(event: string, callback: (event: { payload: T }) => void): Promise<UnlistenFn>;
}

export type { Transport as default };
