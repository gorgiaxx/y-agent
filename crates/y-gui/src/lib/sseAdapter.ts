// SSE adapter for HttpTransport.
//
// Maintains a single EventSource connection to the y-web SSE endpoint and
// dispatches events to registered listeners, matching the Tauri `listen()`
// callback shape `{ payload: T }`.

import type { UnlistenFn, ConnectionStatus } from './transport';

type Callback = (event: { payload: unknown }) => void;

export type { ConnectionStatus };
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
  private eventHandlers = new Map<string, EventListener>();
  private reconnectMs = 1000;
  private maxReconnectMs = 30000;
  private disposed = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
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
    this.eventHandlers.clear();
    for (const event of this.listeners.keys()) {
      this.registerEventListener(event);
    }

    this.source.onopen = () => {
      this.reconnectMs = 1000;
      this.setStatus('connected');
    };

    this.source.onmessage = (ev) => {
      this.handleRaw(ev);
    };

    this.source.onerror = () => {
      this.source?.close();
      this.source = null;
      this.setStatus('disconnected');
      if (!this.disposed) {
        this.reconnectTimer = setTimeout(() => this.connect(), this.reconnectMs);
        this.reconnectMs = Math.min(this.reconnectMs * 2, this.maxReconnectMs);
      }
    };
  }

  private registerEventListener(event: string) {
    if (!this.source || this.eventHandlers.has(event)) return;

    const handler = ((ev: MessageEvent) => {
      try {
        const payload = normalizeSsePayload(JSON.parse(ev.data));
        const cbs = this.listeners.get(event);
        if (cbs) {
          for (const cb of cbs) {
            cb({ payload } as { payload: unknown });
          }
        }
      } catch (e) {
        console.warn('[sse] Failed to parse event data:', e);
      }
    }) as EventListener;

    this.source.addEventListener(event, handler);
    this.eventHandlers.set(event, handler);
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
      this.registerEventListener(event);
    }
    const cb = callback as Callback;
    set.add(cb);

    return () => {
      set!.delete(cb);
      if (set!.size === 0) {
        this.listeners.delete(event);
        const handler = this.eventHandlers.get(event);
        if (handler) {
          this.source?.removeEventListener(event, handler);
          this.eventHandlers.delete(event);
        }
      }
    };
  }

  dispose() {
    this.disposed = true;
    if (this.reconnectTimer != null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.source?.close();
    this.source = null;
    this.eventHandlers.clear();
    this.listeners.clear();
  }
}
