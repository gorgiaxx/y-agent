import { describe, expect, it, vi } from 'vitest';

import {
  buildSessionContextMenuItems,
  buildWorkspaceContextMenuItems,
} from '../components/chat-panel/contextMenuItems';
import {
  buildPlanStepContextMenuItems,
} from '../components/chat-panel/chat-box/tool-renderers/planStepContextMenu';
import type { ContextMenuItem } from '../lib/platform';
import type { SessionInfo, WorkspaceInfo } from '../types';

const session: SessionInfo = {
  id: 'session-1',
  title: 'Session',
  created_at: '2026-07-30T00:00:00Z',
  updated_at: '2026-07-30T00:00:00Z',
  message_count: 2,
};

const workspace: WorkspaceInfo = {
  id: 'workspace-1',
  name: 'Workspace',
  path: '/tmp/workspace',
};

function itemTexts(items: ContextMenuItem[]): string[] {
  return items.flatMap((item) => {
    if (item.kind === 'separator') return [];
    if (item.kind === 'submenu') return [item.text, ...itemTexts(item.items)];
    return [item.text];
  });
}

describe('sidebar context menu items', () => {
  it('builds session actions without importing a host-specific menu API', () => {
    const onRename = vi.fn();
    const onDelete = vi.fn();
    const items = buildSessionContextMenuItems({
      session,
      workspaces: [workspace],
      currentWorkspaceId: workspace.id,
      hasFork: true,
      batchIds: null,
      onAssignSession: vi.fn(),
      onUnassignSession: vi.fn(),
      onRename,
      onFork: vi.fn(),
      onDelete,
      onBatchDelete: vi.fn(),
    });

    expect(itemTexts(items)).toEqual([
      'Move to workspace',
      'Workspace *',
      'Remove from workspace',
      'Rename',
      'Fork session',
      'Delete session',
    ]);

    const rename = items.find((item) => item.kind === 'item' && item.text === 'Rename');
    const deleteItem = items.find((item) => item.kind === 'item' && item.text === 'Delete session');
    if (rename?.kind === 'item') rename.action?.();
    if (deleteItem?.kind === 'item') deleteItem.action?.();

    expect(onRename).toHaveBeenCalledWith(session);
    expect(onDelete).toHaveBeenCalledWith(session.id);
  });

  it('gates workspace reveal actions through platform capabilities', () => {
    const withoutReveal = buildWorkspaceContextMenuItems({
      workspace,
      canReveal: false,
      onEdit: vi.fn(),
      onReveal: vi.fn(),
      onDelete: vi.fn(),
    });
    const withReveal = buildWorkspaceContextMenuItems({
      workspace,
      canReveal: true,
      onEdit: vi.fn(),
      onReveal: vi.fn(),
      onDelete: vi.fn(),
    });

    expect(itemTexts(withoutReveal)).toEqual(['Edit', 'Delete workspace']);
    expect(itemTexts(withReveal)).toEqual(['Edit', 'Open in file manager', 'Delete workspace']);
  });

  it('builds plan retry actions with the shared context-menu contract', () => {
    const onRetryFromHere = vi.fn();
    const items = buildPlanStepContextMenuItems({
      planRunId: 'run-1',
      sessionId: 'session-1',
      task: {
        id: 'task-1',
        phase: 1,
        title: 'Verify changes',
        description: '',
        status: 'failed',
        dependsOn: [],
        estimatedIterations: 1,
        keyFiles: [],
        acceptanceCriteria: [],
      },
      onRetryFromHere,
    });

    expect(itemTexts(items)).toEqual(['Retry from: Verify changes']);
    const retry = items[0];
    if (retry?.kind === 'item') retry.action?.();
    expect(onRetryFromHere).toHaveBeenCalledWith('run-1', 'task-1');
  });
});
