import { describe, expect, it, vi, beforeEach } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: invokeMock,
  },
}));

import { createWorkspaceRecord } from '../hooks/useWorkspaces';

describe('createWorkspaceRecord', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('creates a workspace from the provided form values without reopening the folder picker', async () => {
    invokeMock.mockResolvedValue({
      id: 'workspace-1',
      name: 'demo',
      path: '/tmp/demo',
    });

    const result = await createWorkspaceRecord('demo', '/tmp/demo');

    expect(result).toEqual({
      id: 'workspace-1',
      name: 'demo',
      path: '/tmp/demo',
    });
    expect(invokeMock).toHaveBeenCalledWith('workspace_create', {
      name: 'demo',
      path: '/tmp/demo',
    });
  });
});
