// Hook for rewind functionality -- lists rewind points and executes rewind.
//
// Wraps the Tauri `rewind_list_points` and `rewind_execute` commands
// with loading state and error handling.

import { useState, useCallback } from 'react';
import { transport } from '../lib';

// ---------------------------------------------------------------------------
// Types (mirrors Rust RewindPointInfo / RewindResult)
// ---------------------------------------------------------------------------

export interface DiffStats {
  files_changed: number;
  files_created: number;
  files_deleted: number;
}

export interface RewindPointInfo {
  message_id: string;
  turn_number: number;
  message_preview: string;
  timestamp: number;
  diff_stats: DiffStats;
}

export interface RewindConflict {
  path: string;
  reason: string;
}

export interface RewindReport {
  restored: string[];
  deleted: string[];
  conflicts: RewindConflict[];
}

export interface RewindResult {
  target_message_id: string;
  messages_removed: number;
  checkpoints_invalidated: number;
  file_report: RewindReport;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

interface UseRewindReturn {
  /** Whether the rewind panel is open. */
  isOpen: boolean;
  /** Open the rewind panel (loads points). */
  open: (sessionId: string) => void;
  /** Close the rewind panel. */
  close: () => void;
  /** Available rewind points (most recent first). */
  points: RewindPointInfo[];
  /** Whether points are being loaded. */
  isLoading: boolean;
  /** Execute a rewind to a specific message boundary. */
  executeRewind: (sessionId: string, messageId: string) => Promise<RewindResult | null>;
  /** Whether a rewind is currently executing. */
  isRewinding: boolean;
  /** Error message from the last operation. */
  error: string | null;
}

export function useRewind(): UseRewindReturn {
  const [isOpen, setIsOpen] = useState(false);
  const [points, setPoints] = useState<RewindPointInfo[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isRewinding, setIsRewinding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const open = useCallback(async (sessionId: string) => {
    setIsOpen(true);
    setIsLoading(true);
    setError(null);
    try {
      const result = await transport.invoke<RewindPointInfo[]>('rewind_list_points', {
        sessionId,
      });
      setPoints(result);
    } catch (e) {
      setError(String(e));
      setPoints([]);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const close = useCallback(() => {
    setIsOpen(false);
    setPoints([]);
    setError(null);
  }, []);

  const executeRewind = useCallback(
    async (sessionId: string, messageId: string): Promise<RewindResult | null> => {
      setIsRewinding(true);
      setError(null);
      try {
        const result = await transport.invoke<RewindResult>('rewind_execute', {
          sessionId,
          targetMessageId: messageId,
        });
        setIsOpen(false);
        setPoints([]);
        return result;
      } catch (e) {
        setError(String(e));
        return null;
      } finally {
        setIsRewinding(false);
      }
    },
    [],
  );

  return {
    isOpen,
    open,
    close,
    points,
    isLoading,
    executeRewind,
    isRewinding,
    error,
  };
}
