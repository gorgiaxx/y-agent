// Custom hook for workspace management.

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { WorkspaceInfo } from '../types';

interface UseWorkspacesReturn {
  workspaces: WorkspaceInfo[];
  sessionWorkspaceMap: Record<string, string>;
  createWorkspace: () => Promise<WorkspaceInfo | null>;
  updateWorkspace: (id: string, name: string, path: string) => Promise<void>;
  deleteWorkspace: (id: string) => Promise<void>;
  assignSession: (workspaceId: string, sessionId: string) => Promise<void>;
  unassignSession: (sessionId: string) => Promise<void>;
  refreshWorkspaces: () => Promise<void>;
}

export function useWorkspaces(): UseWorkspacesReturn {
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [sessionWorkspaceMap, setSessionWorkspaceMap] = useState<Record<string, string>>({});

  const refreshWorkspaces = useCallback(async () => {
    try {
      const [list, map] = await Promise.all([
        invoke<WorkspaceInfo[]>('workspace_list'),
        invoke<Record<string, string>>('workspace_session_map'),
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

  const createWorkspace = useCallback(async (): Promise<WorkspaceInfo | null> => {
    // Open native folder picker.
    const selected = await open({ directory: true, multiple: false }).catch(() => null);
    if (!selected || typeof selected !== 'string') return null;

    // Derive a default name from the folder base name.
    const parts = selected.replace(/\\/g, '/').split('/');
    const defaultName = parts[parts.length - 1] || 'Workspace';

    try {
      const ws = await invoke<WorkspaceInfo>('workspace_create', {
        name: defaultName,
        path: selected,
      });
      setWorkspaces((prev) => [...prev, ws]);
      return ws;
    } catch (e) {
      console.error('Failed to create workspace:', e);
      return null;
    }
  }, []);

  const updateWorkspace = useCallback(async (id: string, name: string, path: string) => {
    try {
      await invoke('workspace_update', { id, name, path });
      setWorkspaces((prev) => prev.map((w) => (w.id === id ? { ...w, name, path } : w)));
    } catch (e) {
      console.error('Failed to update workspace:', e);
    }
  }, []);

  const deleteWorkspace = useCallback(async (id: string) => {
    try {
      await invoke('workspace_delete', { id });
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
      await invoke('workspace_assign_session', { workspaceId, sessionId });
      setSessionWorkspaceMap((prev) => ({ ...prev, [sessionId]: workspaceId }));
    } catch (e) {
      console.error('Failed to assign session:', e);
    }
  }, []);

  const unassignSession = useCallback(async (sessionId: string) => {
    try {
      await invoke('workspace_unassign_session', { sessionId });
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
