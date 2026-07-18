import { describe, expect, it, vi, beforeEach } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: invokeMock,
  },
}));

import {
  createWorkspaceRecord,
  getWorkspaceTrust,
  setWorkspaceTrust,
} from '../hooks/useWorkspaces';

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

describe('workspace trust helpers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('loads trust status through the shared transport', async () => {
    invokeMock.mockResolvedValue({
      canonical_path: '/tmp/demo',
      status: 'unknown',
      updated_at: null,
    });

    const result = await getWorkspaceTrust('/tmp/demo');

    expect(result.status).toBe('unknown');
    expect(invokeMock).toHaveBeenCalledWith('workspace_trust_status', {
      path: '/tmp/demo',
    });
  });

  it('persists trusted and untrusted decisions through matching commands', async () => {
    invokeMock.mockResolvedValue({
      canonical_path: '/tmp/demo',
      status: 'trusted',
      updated_at: '2026-07-17T00:00:00Z',
    });

    await setWorkspaceTrust('/tmp/demo', true);
    await setWorkspaceTrust('/tmp/demo', false);

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'workspace_trust', {
      path: '/tmp/demo',
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'workspace_untrust', {
      path: '/tmp/demo',
    });
  });
});
