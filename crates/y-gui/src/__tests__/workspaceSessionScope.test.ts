import { describe, expect, it } from 'vitest';
import { GUI_COMMANDS } from '../commands';
import { resolveCurrentWorkspacePath } from '../hooks/workspaceSessionScope';
import type { SessionInfo, WorkspaceInfo } from '../types';

const workspaces: WorkspaceInfo[] = [
  { id: 'workspace-a', name: 'A', path: '/projects/a' },
  { id: 'workspace-b', name: 'B', path: '/projects/b' },
];

const sessions: SessionInfo[] = [
  {
    id: 'session-a',
    title: 'A session',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    message_count: 1,
    workspace_path: '/canonical/projects/a',
  },
];

describe('workspace-scoped GUI resume', () => {
  it('registers resume as an immediate GUI command', () => {
    expect(GUI_COMMANDS.find((command) => command.name === 'resume')).toMatchObject({
      immediate: true,
      category: 'Session',
    });
  });

  it('uses the active session persisted workspace before the welcome selection', () => {
    expect(resolveCurrentWorkspacePath({
      activeSessionId: 'session-a',
      sessions,
      welcomeWorkspaceId: 'workspace-b',
      workspaces,
    })).toBe('/canonical/projects/a');
  });

  it('uses an explicitly selected welcome workspace when no session is active', () => {
    expect(resolveCurrentWorkspacePath({
      activeSessionId: null,
      sessions,
      welcomeWorkspaceId: 'workspace-b',
      workspaces,
    })).toBe('/projects/b');
  });

  it('does not fall back to a global or first workspace', () => {
    expect(resolveCurrentWorkspacePath({
      activeSessionId: null,
      sessions,
      welcomeWorkspaceId: null,
      workspaces,
    })).toBeNull();
  });
});
