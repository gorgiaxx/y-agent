import { vi } from 'vitest';

class MockEventSource {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readonly url: string;
  readyState = MockEventSource.CONNECTING;
  withCredentials = false;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(url: string | URL) {
    this.url = String(url);
  }

  addEventListener() {}

  removeEventListener() {}

  dispatchEvent() {
    return true;
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }
}

vi.stubGlobal('EventSource', MockEventSource);
