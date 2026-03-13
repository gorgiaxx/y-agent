// Hook: polls observability_snapshot while the panel is open.

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SystemSnapshot } from '../types';

const POLL_INTERVAL_MS = 2000;

export function useObservability(active: boolean) {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [loading, setLoading] = useState(false);

  const fetchSnapshot = useCallback(async () => {
    try {
      const snap = await invoke<SystemSnapshot>('observability_snapshot');
      setSnapshot(snap);
    } catch (err) {
      console.error('[useObservability] fetch failed:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!active) {
      return;
    }

    setLoading(true);
    fetchSnapshot();

    const id = setInterval(fetchSnapshot, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [active, fetchSnapshot]);

  return { snapshot, loading };
}
