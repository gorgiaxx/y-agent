import { useCallback, useEffect, useRef, useState } from 'react';

import { logger, transport } from '../lib';

export type BackgroundTaskStatus = 'running' | 'completed' | 'failed' | 'unknown';

export interface BackgroundTaskInfo {
  process_id: string;
  backend: string;
  command: string;
  working_dir: string | null;
  status: BackgroundTaskStatus;
  exit_code: number | null;
  error: string | null;
  duration_ms: number;
}

export interface BackgroundTaskSnapshot {
  process_id: string;
  backend: string;
  status: BackgroundTaskStatus;
  exit_code: number | null;
  error: string | null;
  stdout: string;
  stderr: string;
  duration_ms: number;
}

export type BackgroundTaskLogStream = 'stdout' | 'stderr';

export interface BackgroundTaskLogEntry {
  id: string;
  stream: BackgroundTaskLogStream;
  content: string;
  timestamp: number;
}

const POLL_YIELD_MS = 100;
const MAX_OUTPUT_BYTES = 64 * 1024;
const LOG_BUFFER_LIMIT = 128 * 1024;
const REFRESH_INTERVAL_MS = 5_000;
let logEntrySequence = 0;

function nextLogEntryId(processId: string, stream: BackgroundTaskLogStream): string {
  logEntrySequence += 1;
  return `${processId}-${stream}-${logEntrySequence}`;
}

function appendBoundedLogs(
  current: BackgroundTaskLogEntry[],
  next: BackgroundTaskLogEntry[],
): BackgroundTaskLogEntry[] {
  if (next.length === 0) return current;
  const combined = [...current, ...next];
  let totalLength = combined.reduce((total, entry) => total + entry.content.length, 0);

  while (combined.length > 1 && totalLength > LOG_BUFFER_LIMIT) {
    const removed = combined.shift();
    totalLength -= removed?.content.length ?? 0;
  }

  return combined;
}

function logEntriesFromSnapshot(snapshot: BackgroundTaskSnapshot): BackgroundTaskLogEntry[] {
  const timestamp = Date.now();
  const entries: BackgroundTaskLogEntry[] = [];
  if (snapshot.stdout) {
    entries.push({
      id: nextLogEntryId(snapshot.process_id, 'stdout'),
      stream: 'stdout',
      content: snapshot.stdout,
      timestamp,
    });
  }
  if (snapshot.stderr) {
    entries.push({
      id: nextLogEntryId(snapshot.process_id, 'stderr'),
      stream: 'stderr',
      content: snapshot.stderr,
      timestamp,
    });
  }
  return entries;
}

function taskFromSnapshot(
  snapshot: BackgroundTaskSnapshot,
  existing?: BackgroundTaskInfo,
): BackgroundTaskInfo {
  return {
    process_id: snapshot.process_id,
    backend: snapshot.backend,
    command: existing?.command ?? snapshot.process_id,
    working_dir: existing?.working_dir ?? null,
    status: snapshot.status,
    exit_code: snapshot.exit_code,
    error: snapshot.error,
    duration_ms: snapshot.duration_ms,
  };
}

export function useBackgroundTasks(sessionId: string | null) {
  const [tasks, setTasks] = useState<BackgroundTaskInfo[]>([]);
  const [logs, setLogs] = useState<Record<string, BackgroundTaskLogEntry[]>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyProcessId, setBusyProcessId] = useState<string | null>(null);
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;

  const refresh = useCallback(async () => {
    if (!sessionId) {
      setTasks([]);
      setError(null);
      setLoading(false);
      return;
    }
    const requestedSessionId = sessionId;
    setLoading(true);
    try {
      const next = await transport.invoke<BackgroundTaskInfo[]>('background_task_list', {
        sessionId,
      });
      if (sessionIdRef.current !== requestedSessionId) return;
      setTasks(next);
      setError(null);
    } catch (e) {
      if (sessionIdRef.current !== requestedSessionId) return;
      const message = String(e);
      setError(message);
      logger.warn('[background-tasks] list failed:', message);
    } finally {
      if (sessionIdRef.current === requestedSessionId) {
        setLoading(false);
      }
    }
  }, [sessionId]);

  const applySnapshot = useCallback((snapshot: BackgroundTaskSnapshot) => {
    const entries = logEntriesFromSnapshot(snapshot);
    if (entries.length > 0) {
      setLogs((prev) => {
        const existing = prev[snapshot.process_id] ?? [];
        return {
          ...prev,
          [snapshot.process_id]: appendBoundedLogs(existing, entries),
        };
      });
    }

    setTasks((prev) => {
      const existing = prev.find((task) => task.process_id === snapshot.process_id);
      const nextTask = taskFromSnapshot(snapshot, existing);
      if (!existing) return [nextTask, ...prev];
      return prev.map((task) => (
        task.process_id === snapshot.process_id ? nextTask : task
      ));
    });
  }, []);

  const runSnapshotAction = useCallback(async (
    processId: string,
    command: () => Promise<BackgroundTaskSnapshot>,
  ) => {
    const requestedSessionId = sessionIdRef.current;
    setBusyProcessId(processId);
    try {
      const snapshot = await command();
      if (sessionIdRef.current !== requestedSessionId) return null;
      applySnapshot(snapshot);
      setError(null);
      return snapshot;
    } catch (e) {
      if (sessionIdRef.current !== requestedSessionId) return null;
      const message = String(e);
      setError(message);
      logger.warn('[background-tasks] action failed:', message);
      return null;
    } finally {
      if (sessionIdRef.current === requestedSessionId) {
        setBusyProcessId(null);
      }
    }
  }, [applySnapshot]);

  const pollTask = useCallback((processId: string) => runSnapshotAction(
    processId,
    () => {
      if (!sessionId) return Promise.reject(new Error('sessionId is required'));
      return transport.invoke<BackgroundTaskSnapshot>('background_task_poll', {
        sessionId,
        processId,
        yieldTimeMs: POLL_YIELD_MS,
        maxOutputBytes: MAX_OUTPUT_BYTES,
      });
    },
  ), [runSnapshotAction, sessionId]);

  const killTask = useCallback((processId: string) => runSnapshotAction(
    processId,
    () => {
      if (!sessionId) return Promise.reject(new Error('sessionId is required'));
      return transport.invoke<BackgroundTaskSnapshot>('background_task_kill', {
        sessionId,
        processId,
        yieldTimeMs: POLL_YIELD_MS,
        maxOutputBytes: MAX_OUTPUT_BYTES,
      });
    },
  ), [runSnapshotAction, sessionId]);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    setTasks([]);
    setLogs({});
    setError(null);
    setBusyProcessId(null);
  }, [sessionId]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => {
      void refresh();
    }, REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  return {
    tasks,
    logs,
    loading,
    error,
    busyProcessId,
    refresh,
    pollTask,
    killTask,
  };
}

export type UseBackgroundTasksReturn = ReturnType<typeof useBackgroundTasks>;
