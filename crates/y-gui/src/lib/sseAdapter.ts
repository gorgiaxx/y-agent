// SSE adapter for HttpTransport.
//
// Maintains a single EventSource connection to the y-web SSE endpoint and
// dispatches events to registered listeners, matching the Tauri `listen()`
// callback shape `{ payload: T }`.

import type { UnlistenFn } from './transport';

type Callback = (event: { payload: unknown }) => void;

export type ConnectionStatus = 'connected' | 'connecting' | 'disconnected';
type StatusCallback = (status: ConnectionStatus) => void;

function normalizeSsePayload(payload: unknown) {
  if (
    payload
    && typeof payload === 'object'
    && 'type' in payload
    && 'data' in payload
  ) {
    return (payload as { data: unknown }).data;
  }
  return payload;
}

export class SseAdapter {
  private url: string;
  private token: string | null;
  private source: EventSource | null = null;
  private listeners = new Map<string, Set<Callback>>();
  private reconnectMs = 1000;
  private maxReconnectMs = 30000;
  private disposed = false;
  private _status: ConnectionStatus = 'connecting';
  private statusListeners = new Set<StatusCallback>();

  constructor(baseUrl: string, token: string | null = null) {
    this.url = `${baseUrl}/api/v1/events`;
    this.token = token;
    this.connect();
  }

  get status(): ConnectionStatus {
    return this._status;
  }

  onStatusChange(cb: StatusCallback): () => void {
    this.statusListeners.add(cb);
    return () => this.statusListeners.delete(cb);
  }

  private setStatus(s: ConnectionStatus) {
    if (this._status === s) return;
    this._status = s;
    for (const cb of this.statusListeners) cb(s);
  }

  private connect() {
    if (this.disposed) return;
    this.setStatus('connecting');

    const url = this.token ? `${this.url}?token=${encodeURIComponent(this.token)}` : this.url;
    this.source = new EventSource(url);

    this.source.onopen = () => {
      this.reconnectMs = 1000;
      this.setStatus('connected');
      this.reregisterListeners();
    };

    this.source.onmessage = (ev) => {
      this.handleRaw(ev);
    };

    this.source.onerror = () => {
      this.source?.close();
      this.source = null;
      this.setStatus('disconnected');
      if (!this.disposed) {
        setTimeout(() => this.connect(), this.reconnectMs);
        this.reconnectMs = Math.min(this.reconnectMs * 2, this.maxReconnectMs);
      }
    };
  }

  private reregisterListeners() {
    if (!this.source) return;
    for (const event of this.listeners.keys()) {
      this.source.addEventListener(event, ((ev: MessageEvent) => {
        try {
          const payload = normalizeSsePayload(JSON.parse(ev.data));
          const cbs = this.listeners.get(event);
          if (cbs) {
            for (const cb of cbs) {
              cb({ payload } as { payload: unknown });
            }
          }
        } catch { /* ignore */ }
      }) as EventListener);
    }
  }

  private handleRaw(ev: MessageEvent) {
    try {
      const data = JSON.parse(ev.data) as { event?: string; [key: string]: unknown };
      const eventName = data.event as string | undefined;
      if (!eventName) return;

      const callbacks = this.listeners.get(eventName);
      if (!callbacks) return;

      const payload = normalizeSsePayload(data);
      for (const cb of callbacks) {
        cb({ payload });
      }
    } catch {
      // ignore malformed SSE data
    }
  }

  listen<T = unknown>(event: string, callback: (event: { payload: T }) => void): UnlistenFn {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);

      // Register a named event listener on the EventSource so the browser
      // dispatches events with `event: <name>` lines directly to us.
      this.source?.addEventListener(event, ((ev: MessageEvent) => {
        try {
          const payload = normalizeSsePayload(JSON.parse(ev.data)) as T;
          const cbs = this.listeners.get(event);
          if (cbs) {
            for (const cb of cbs) {
              cb({ payload } as { payload: unknown });
            }
          }
        } catch { /* ignore */ }
      }) as EventListener);
    }
    const cb = callback as Callback;
    set.add(cb);

    return () => {
      set!.delete(cb);
      if (set!.size === 0) {
        this.listeners.delete(event);
      }
    };
  }

  dispose() {
    this.disposed = true;
    this.source?.close();
    this.source = null;
    this.listeners.clear();
  }
}
