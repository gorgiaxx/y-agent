import { afterEach, describe, expect, it, vi } from 'vitest';
import { SseAdapter } from '../lib/sseAdapter';

class MockEventSource {
  static instances: MockEventSource[] = [];

  readonly url: string | URL;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  private listeners = new Map<string, Set<(event: MessageEvent) => void>>();

  constructor(url: string | URL) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  addEventListener(event: string, callback: EventListener) {
    const callbacks = this.listeners.get(event) ?? new Set();
    callbacks.add(callback as (event: MessageEvent) => void);
    this.listeners.set(event, callbacks);
  }

  removeEventListener() {}

  dispatchEvent() {
    return true;
  }

  close() {}

  emitNamed(event: string, data: unknown) {
    const message = new MessageEvent(event, { data: JSON.stringify(data) });
    for (const callback of this.listeners.get(event) ?? []) {
      callback(message);
    }
  }
}

describe('SseAdapter contract mapping', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    MockEventSource.instances = [];
  });

  it('unwraps y-web adjacently tagged SSE events to Tauri payload shape', () => {
    vi.stubGlobal('EventSource', MockEventSource);
    const adapter = new SseAdapter('http://localhost:3000');
    const payloads: unknown[] = [];

    adapter.listen('chat:started', (event) => {
      payloads.push(event.payload);
    });

    MockEventSource.instances[0].emitNamed('chat:started', {
      type: 'ChatStarted',
      data: { run_id: 'r1', session_id: 's1' },
    });

    expect(payloads).toEqual([{ run_id: 'r1', session_id: 's1' }]);
    adapter.dispose();
  });

  it('keeps raw named payloads when the server already sends Tauri-shaped data', () => {
    vi.stubGlobal('EventSource', MockEventSource);
    const adapter = new SseAdapter('http://localhost:3000');
    const payloads: unknown[] = [];

    adapter.listen('session:title_updated', (event) => {
      payloads.push(event.payload);
    });

    MockEventSource.instances[0].emitNamed('session:title_updated', {
      session_id: 's1',
      title: 'Project Notes',
    });

    expect(payloads).toEqual([{ session_id: 's1', title: 'Project Notes' }]);
    adapter.dispose();
  });
});
