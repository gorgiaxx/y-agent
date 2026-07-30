import type { ContextMenuItem } from '../../lib/platform';
import type { SessionInfo, WorkspaceInfo } from '../../types';

export interface SessionMenuOptions {
  session: SessionInfo;
  workspaces: WorkspaceInfo[];
  currentWorkspaceId: string | null;
  hasFork: boolean;
  batchIds: string[] | null;
  onAssignSession: (workspaceId: string, sessionId: string) => void;
  onUnassignSession: (sessionId: string) => void;
  onRename: (session: SessionInfo) => void;
  onFork: (sessionId: string) => void;
  onDelete: (sessionId: string) => void;
  onBatchDelete: () => void;
}

export function buildSessionContextMenuItems(opts: SessionMenuOptions): ContextMenuItem[] {
  const {
    session,
    workspaces,
    currentWorkspaceId,
    hasFork,
    batchIds,
    onAssignSession,
    onUnassignSession,
    onRename,
    onFork,
    onDelete,
    onBatchDelete,
  } = opts;

  const isBatch = batchIds !== null && batchIds.length > 1;
  const items: ContextMenuItem[] = [];

  if (isBatch) {
    items.push({
      kind: 'item',
      text: `${batchIds.length} selected`,
      enabled: false,
    });
    items.push({ kind: 'separator' });
  }

  if (workspaces.length > 0) {
    items.push({
      kind: 'submenu',
      text: 'Move to workspace',
      items: workspaces.map((workspace) => ({
        kind: 'item',
        text: workspace.id === currentWorkspaceId && !isBatch
          ? `${workspace.name} *`
          : workspace.name,
        action: () => {
          if (isBatch) {
            for (const id of batchIds) onAssignSession(workspace.id, id);
          } else {
            onAssignSession(workspace.id, session.id);
          }
        },
      })),
    });

    if (currentWorkspaceId !== null || isBatch) {
      items.push({
        kind: 'item',
        text: 'Remove from workspace',
        action: () => {
          if (isBatch) {
            for (const id of batchIds) onUnassignSession(id);
          } else {
            onUnassignSession(session.id);
          }
        },
      });
    }

    items.push({ kind: 'separator' });
  }

  if (!isBatch) {
    items.push({
      kind: 'item',
      text: 'Rename',
      action: () => onRename(session),
    });

    if (hasFork) {
      items.push({
        kind: 'item',
        text: 'Fork session',
        action: () => onFork(session.id),
      });
    }

    items.push({ kind: 'separator' });
  }

  items.push({
    kind: 'item',
    text: isBatch ? `Delete ${batchIds.length} sessions` : 'Delete session',
    action: () => {
      if (isBatch) onBatchDelete();
      else onDelete(session.id);
    },
  });

  return items;
}

export interface WorkspaceMenuOptions {
  workspace: WorkspaceInfo;
  canReveal: boolean;
  onEdit: (workspace: WorkspaceInfo) => void;
  onReveal: (path: string) => void;
  onDelete: (workspaceId: string) => void;
}

export function buildWorkspaceContextMenuItems(opts: WorkspaceMenuOptions): ContextMenuItem[] {
  const { workspace, canReveal, onEdit, onReveal, onDelete } = opts;
  const items: ContextMenuItem[] = [
    {
      kind: 'item',
      text: 'Edit',
      action: () => onEdit(workspace),
    },
  ];

  if (canReveal) {
    items.push({
      kind: 'item',
      text: 'Open in file manager',
      action: () => onReveal(workspace.path),
    });
  }

  items.push({ kind: 'separator' });
  items.push({
    kind: 'item',
    text: 'Delete workspace',
    action: () => onDelete(workspace.id),
  });

  return items;
}
