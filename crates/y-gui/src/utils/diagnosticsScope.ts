import type { ViewType } from '../types';

export interface DiagnosticsScope {
  isGlobal: boolean;
  sessionId: string | null;
}

export function resolveDiagnosticsScope(
  activeView: ViewType,
  activeSessionId: string | null,
): DiagnosticsScope {
  if (activeView === 'chat' && activeSessionId) {
    return {
      isGlobal: false,
      sessionId: activeSessionId,
    };
  }

  return {
    isGlobal: true,
    sessionId: null,
  };
}
