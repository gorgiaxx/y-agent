import type { SessionInfo, WorkspaceInfo } from '../types';

interface WorkspaceSessionScopeInput {
  activeSessionId: string | null;
  sessions: SessionInfo[];
  welcomeWorkspaceId: string | null;
  workspaces: WorkspaceInfo[];
}

/** Resolve the explicit workspace used by GUI session creation and resume. */
export function resolveCurrentWorkspacePath({
  activeSessionId,
  sessions,
  welcomeWorkspaceId,
  workspaces,
}: WorkspaceSessionScopeInput): string | null {
  if (activeSessionId) {
    return sessions.find((session) => session.id === activeSessionId)?.workspace_path ?? null;
  }
  if (!welcomeWorkspaceId) return null;
  return workspaces.find((workspace) => workspace.id === welcomeWorkspaceId)?.path ?? null;
}
