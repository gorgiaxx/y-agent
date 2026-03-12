// Custom hook for session management.

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SessionInfo } from '../types';

interface UseSessionsReturn {
  sessions: SessionInfo[];
  activeSessionId: string | null;
  loading: boolean;
  createSession: (title?: string) => Promise<SessionInfo | null>;
  selectSession: (id: string) => void;
  deleteSession: (id: string) => Promise<void>;
  refreshSessions: () => Promise<void>;
}

export function useSessions(): UseSessionsReturn {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refreshSessions = useCallback(async () => {
    try {
      const list = await invoke<SessionInfo[]>('session_list');
      setSessions(list);
    } catch (e) {
      console.error('Failed to load sessions:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshSessions();
  }, [refreshSessions]);

  const createSession = useCallback(
    async (title?: string): Promise<SessionInfo | null> => {
      try {
        const session = await invoke<SessionInfo>('session_create', {
          title: title ?? null,
        });
        setSessions((prev) => [session, ...prev]);
        setActiveSessionId(session.id);
        return session;
      } catch (e) {
        console.error('Failed to create session:', e);
        return null;
      }
    },
    [],
  );

  const selectSession = useCallback((id: string) => {
    setActiveSessionId(id);
  }, []);

  const deleteSession = useCallback(
    async (id: string) => {
      try {
        await invoke('session_delete', { sessionId: id });
        setSessions((prev) => prev.filter((s) => s.id !== id));
        if (activeSessionId === id) {
          setActiveSessionId(null);
        }
      } catch (e) {
        console.error('Failed to delete session:', e);
      }
    },
    [activeSessionId],
  );

  return {
    sessions,
    activeSessionId,
    loading,
    createSession,
    selectSession,
    deleteSession,
    refreshSessions,
  };
}
