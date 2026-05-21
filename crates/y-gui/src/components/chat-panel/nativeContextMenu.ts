import { Menu } from '@tauri-apps/api/menu';
import type { PredefinedMenuItemOptions, MenuItemOptions, SubmenuOptions } from '@tauri-apps/api/menu';
import type { SessionInfo, WorkspaceInfo } from '../../types';

type MenuEntry = MenuItemOptions | PredefinedMenuItemOptions | SubmenuOptions;

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

export async function showSessionContextMenu(opts: SessionMenuOptions): Promise<void> {
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
  const items: MenuEntry[] = [];

  if (isBatch) {
    items.push({ text: `${batchIds.length} selected`, enabled: false });
    items.push({ item: 'Separator' });
  }

  if (workspaces.length > 0) {
    const wsItems: MenuItemOptions[] = workspaces.map((ws) => ({
      text: ws.id === currentWorkspaceId && !isBatch ? `${ws.name} *` : ws.name,
      action: () => {
        if (isBatch) {
          for (const id of batchIds) onAssignSession(ws.id, id);
        } else {
          onAssignSession(ws.id, session.id);
        }
      },
    }));

    items.push({
      text: 'Move to workspace',
      items: wsItems,
    } as SubmenuOptions);

    const hasAssigned = isBatch
      ? batchIds.some((id) => id === session.id ? currentWorkspaceId !== null : false)
      : currentWorkspaceId !== null;

    if (hasAssigned || (isBatch && batchIds.length > 0)) {
      items.push({
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

    items.push({ item: 'Separator' });
  }

  if (!isBatch) {
    items.push({
      text: 'Rename',
      action: () => onRename(session),
    });

    if (hasFork) {
      items.push({
        text: 'Fork session',
        action: () => onFork(session.id),
      });
    }

    items.push({ item: 'Separator' });
  }

  items.push({
    text: isBatch ? `Delete ${batchIds.length} sessions` : 'Delete session',
    action: () => {
      if (isBatch) {
        onBatchDelete();
      } else {
        onDelete(session.id);
      }
    },
  });

  const menu = await Menu.new({ items });
  await menu.popup();
}

export interface WorkspaceMenuOptions {
  workspace: WorkspaceInfo;
  canReveal: boolean;
  onEdit: (workspace: WorkspaceInfo) => void;
  onReveal: (path: string) => void;
  onDelete: (workspaceId: string) => void;
}

export async function showWorkspaceContextMenu(opts: WorkspaceMenuOptions): Promise<void> {
  const { workspace, canReveal, onEdit, onReveal, onDelete } = opts;

  const items: MenuEntry[] = [
    {
      text: 'Edit',
      action: () => onEdit(workspace),
    },
  ];

  if (canReveal) {
    items.push({
      text: 'Open in file manager',
      action: () => onReveal(workspace.path),
    });
  }

  items.push({ item: 'Separator' });
  items.push({
    text: 'Delete workspace',
    action: () => onDelete(workspace.id),
  });

  const menu = await Menu.new({ items });
  await menu.popup();
}
