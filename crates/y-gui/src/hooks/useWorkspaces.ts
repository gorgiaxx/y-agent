// Custom hook for workspace management.

import { useState, useCallback, useEffect } from 'react';
import { transport } from '../lib';
import type { WorkspaceInfo } from '../types';

interface UseWorkspacesReturn {
  workspaces: WorkspaceInfo[];
  sessionWorkspaceMap: Record<string, string>;
  createWorkspace: (name: string, path: string) => Promise<WorkspaceInfo | null>;
  updateWorkspace: (id: string, name: string, path: string) => Promise<void>;
  deleteWorkspace: (id: string) => Promise<void>;
  assignSession: (workspaceId: string, sessionId: string) => Promise<void>;
  unassignSession: (sessionId: string) => Promise<void>;
  refreshWorkspaces: () => Promise<void>;
}

export async function createWorkspaceRecord(
  name: string,
  path: string,
): Promise<WorkspaceInfo | null> {
  try {
    return await transport.invoke<WorkspaceInfo>('workspace_create', { name, path });
  } catch (e) {
    console.error('Failed to create workspace:', e);
    return null;
  }
}

export function useWorkspaces(): UseWorkspacesReturn {
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [sessionWorkspaceMap, setSessionWorkspaceMap] = useState<Record<string, string>>({});

  const refreshWorkspaces = useCallback(async () => {
    try {
      const [list, map] = await Promise.all([
        transport.invoke<WorkspaceInfo[]>('workspace_list'),
        transport.invoke<Record<string, string>>('workspace_session_map'),
      ]);
      setWorkspaces(list);
      setSessionWorkspaceMap(map);
    } catch (e) {
      console.error('Failed to load workspaces:', e);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    refreshWorkspaces();
  }, [refreshWorkspaces]);

  const createWorkspace = useCallback(async (name: string, path: string): Promise<WorkspaceInfo | null> => {
    const ws = await createWorkspaceRecord(name, path);
    if (!ws) return null;
    setWorkspaces((prev) => [...prev, ws]);
    return ws;
  }, []);

  const updateWorkspace = useCallback(async (id: string, name: string, path: string) => {
    try {
      await transport.invoke('workspace_update', { id, name, path });
      setWorkspaces((prev) => prev.map((w) => (w.id === id ? { ...w, name, path } : w)));
    } catch (e) {
      console.error('Failed to update workspace:', e);
    }
  }, []);

  const deleteWorkspace = useCallback(async (id: string) => {
    try {
      await transport.invoke('workspace_delete', { id });
      setWorkspaces((prev) => prev.filter((w) => w.id !== id));
      setSessionWorkspaceMap((prev) => {
        const next = { ...prev };
        for (const sid of Object.keys(next)) {
          if (next[sid] === id) delete next[sid];
        }
        return next;
      });
    } catch (e) {
      console.error('Failed to delete workspace:', e);
    }
  }, []);

  const assignSession = useCallback(async (workspaceId: string, sessionId: string) => {
    try {
      await transport.invoke('workspace_assign_session', { workspaceId, sessionId });
      setSessionWorkspaceMap((prev) => ({ ...prev, [sessionId]: workspaceId }));
    } catch (e) {
      console.error('Failed to assign session:', e);
    }
  }, []);

  const unassignSession = useCallback(async (sessionId: string) => {
    try {
      await transport.invoke('workspace_unassign_session', { sessionId });
      setSessionWorkspaceMap((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
    } catch (e) {
      console.error('Failed to unassign session:', e);
    }
  }, []);

  return {
    workspaces,
    sessionWorkspaceMap,
    createWorkspace,
    updateWorkspace,
    deleteWorkspace,
    assignSession,
    unassignSession,
    refreshWorkspaces,
  };
}
