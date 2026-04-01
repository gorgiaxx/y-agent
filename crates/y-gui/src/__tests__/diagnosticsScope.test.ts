import { describe, expect, it } from 'vitest';

import { resolveDiagnosticsScope } from '../utils/diagnosticsScope';

describe('resolveDiagnosticsScope', () => {
  it('uses the active session in chat view', () => {
    expect(resolveDiagnosticsScope('chat', 'session-123')).toEqual({
      isGlobal: false,
      sessionId: 'session-123',
    });
  });

  it('falls back to global diagnostics when chat has no active session', () => {
    expect(resolveDiagnosticsScope('chat', null)).toEqual({
      isGlobal: true,
      sessionId: null,
    });
  });

  it('uses global diagnostics outside chat view', () => {
    expect(resolveDiagnosticsScope('skills', 'session-123')).toEqual({
      isGlobal: true,
      sessionId: null,
    });
  });
});
