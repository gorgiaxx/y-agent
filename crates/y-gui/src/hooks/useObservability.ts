// React hook for observability data with time-range support.
//
// Polls the live snapshot for real-time data (concurrency, agent pool) and
// queries the persistent store for historical provider metrics when a time
// range is selected.

import { useState, useEffect, useRef, useCallback } from 'react';
import { transport } from '../lib';
import type { SystemSnapshot } from '../types';

export type TimeRange = '15m' | '30m' | '1h' | '6h' | '24h' | 'all';

const RANGE_MS: Record<TimeRange, number | null> = {
  '15m': 15 * 60 * 1000,
  '30m': 30 * 60 * 1000,
  '1h': 60 * 60 * 1000,
  '6h': 6 * 60 * 60 * 1000,
  '24h': 24 * 60 * 60 * 1000,
  all: null,
};

/** Compute an ISO 8601 `since` string for the given range, or null for "all". */
function computeSince(range: TimeRange): string | null {
  const ms = RANGE_MS[range];
  if (ms == null) return null;
  return new Date(Date.now() - ms).toISOString();
}

export interface UseObservabilityOptions {
  /** Whether the panel is currently active/visible. */
  active: boolean;
  /** Polling interval in milliseconds (default: 2000). */
  intervalMs?: number;
  /** Selected time range for historical metrics (default: 'all'). */
  timeRange?: TimeRange;
}

export interface UseObservabilityResult {
  snapshot: SystemSnapshot | null;
  loading: boolean;
  error: string | null;
}

/**
 * Poll observability data.
 *
 * When a `timeRange` is set, provider metrics come from the persistent store
 * (aggregated over the time window). Live data (concurrency, agents) is always
 * real-time from the snapshot endpoint.
 */
export function useObservability({
  active,
  intervalMs = 2000,
  timeRange = 'all',
}: UseObservabilityOptions): UseObservabilityResult {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchData = useCallback(async () => {
    try {
      setError(null);
      // Always fetch the live snapshot for concurrency + agent pool.
      const liveSnap = await transport.invoke<SystemSnapshot>('observability_snapshot');

      // Try to fetch historical metrics from the persistent store.
      // If this fails (e.g. DB issue), fall back to the live snapshot alone.
      let histSnap: SystemSnapshot | null = null;
      try {
        const since = computeSince(timeRange);
        histSnap = await transport.invoke<SystemSnapshot>('observability_history', {
          since,
          until: null,
        });
      } catch (histErr) {
        console.warn('observability history fetch failed, using live data:', histErr);
      }

      if (histSnap) {
        // Merge: use historical metrics for providers, but keep live
        // concurrency (active_requests) and agent pool from the live snapshot.
        const mergedProviders = histSnap.providers.map((histP) => {
          const liveP = liveSnap.providers.find((p) => p.id === histP.id);
          return {
            ...histP,
            // Override with live concurrency data.
            active_requests: liveP?.active_requests ?? histP.active_requests,
            is_frozen: liveP?.is_frozen ?? histP.is_frozen,
            freeze_reason: liveP?.freeze_reason ?? histP.freeze_reason,
          };
        });

        setSnapshot({
          ...liveSnap,
          providers: mergedProviders,
        });
      } else {
        // No historical data available -- show live snapshot as-is.
        setSnapshot(liveSnap);
      }
    } catch (err) {
      console.error('observability snapshot failed:', err);
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [timeRange]);

  useEffect(() => {
    if (!active) {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      return;
    }

    setLoading(true);
    fetchData();
    intervalRef.current = setInterval(fetchData, intervalMs);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [active, intervalMs, fetchData]);

  return { snapshot, loading, error };
}
