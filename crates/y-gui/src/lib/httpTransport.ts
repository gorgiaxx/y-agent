// HTTP transport implementation.
//
// Routes `invoke()` calls to y-web REST endpoints via fetch, and `listen()`
// calls to the SSE adapter. Used when VITE_BACKEND=http.

import type { Transport, UnlistenFn } from './transport';
import { COMMAND_MAP } from './commandMap';
import { SseAdapter } from './sseAdapter';
import type { ConnectionStatus } from './sseAdapter';

const LIFECYCLE_NOOP_COMMANDS = new Set([
  'show_window', 'toggle_devtools', 'window_set_decorations',
  'window_minimize', 'window_toggle_maximize', 'window_close',
  'window_set_theme', 'heartbeat_pong',
]);

const UNSUPPORTED_WEB_COMMANDS = new Set([
  'skill_open_folder',
]);

const GUI_CONFIG_KEY = 'y-agent-gui-config';

export class HttpTransport implements Transport {
  private baseUrl: string;
  private token: string | null;
  private sse: SseAdapter;

  constructor(baseUrl: string, token: string | null = null) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    this.token = token;
    this.sse = new SseAdapter(this.baseUrl, token);
  }

  async invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (LIFECYCLE_NOOP_COMMANDS.has(command)) {
      return undefined as T;
    }

    if (UNSUPPORTED_WEB_COMMANDS.has(command)) {
      throw new Error(`[HttpTransport] Command "${command}" is not supported in the web backend`);
    }

    if (command === 'config_get_gui') {
      const stored = localStorage.getItem(GUI_CONFIG_KEY);
      return (stored ? JSON.parse(stored) : null) as T;
    }

    if (command === 'config_set_gui') {
      const config = args?.config ?? args;
      localStorage.setItem(GUI_CONFIG_KEY, JSON.stringify(config));
      return undefined as T;
    }

    const def = COMMAND_MAP[command];
    if (!def) {
      throw new Error(`[HttpTransport] Unknown command: ${command}`);
    }

    const safeArgs = args ?? {};
    const path = typeof def.path === 'function' ? def.path(safeArgs) : def.path;
    const queryParams = def.query?.(safeArgs);
    const body = def.body?.(safeArgs);

    let url = `${this.baseUrl}${path}`;
    if (queryParams) {
      const filtered = Object.entries(queryParams).filter(([, v]) => v !== undefined) as [string, string][];
      if (filtered.length > 0) {
        const qs = new URLSearchParams(filtered).toString();
        url += `?${qs}`;
      }
    }

    const headers: Record<string, string> = {};
    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`;
    }

    const hasBody = body !== undefined && def.method !== 'GET';
    if (hasBody) {
      headers['Content-Type'] = 'application/json';
    }

    const resp = await fetch(url, {
      method: def.method,
      headers,
      body: hasBody ? JSON.stringify(body) : undefined,
    });

    if (!resp.ok) {
      const text = await resp.text().catch(() => resp.statusText);
      throw new Error(text || `HTTP ${resp.status}`);
    }

    const contentType = resp.headers.get('content-type') ?? '';
    let result: unknown;
    if (contentType.includes('application/json')) {
      result = await resp.json();
      return (def.response ? def.response(result) : result) as T;
    }

    const text = await resp.text();
    if (text === '') return undefined as T;

    try {
      result = JSON.parse(text);
    } catch {
      result = text;
    }
    return (def.response ? def.response(result) : result) as T;
  }

  async listen<T = unknown>(
    event: string,
    callback: (event: { payload: T }) => void,
  ): Promise<UnlistenFn> {
    return this.sse.listen(event, callback);
  }

  get connectionStatus(): ConnectionStatus {
    return this.sse.status;
  }

  onConnectionStatusChange(cb: (status: ConnectionStatus) => void): () => void {
    return this.sse.onStatusChange(cb);
  }

  dispose() {
    this.sse.dispose();
  }
}
