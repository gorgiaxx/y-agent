import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../lib';
import type {
  KnowledgeCollectionInfo,
  KnowledgeEntryInfo,
  KnowledgeEntryDetail,
  KnowledgeSearchResult,
  KnowledgeIngestResult,
  KnowledgeStats,
} from '../types';
import {
  buildKnowledgeIngestBatchPayload,
  buildKnowledgeIngestPayload,
} from './knowledgeInvokePayload';

export type KbIngestStatus = 'idle' | 'ingesting' | 'success' | 'error';
export type KbBatchProgress = { current: number; total: number };

export interface UseKnowledgeReturn {
  collections: KnowledgeCollectionInfo[];
  entries: KnowledgeEntryInfo[];
  selectedCollection: string | null;
  stats: KnowledgeStats | null;
  loading: boolean;
  ingestStatus: KbIngestStatus;
  ingestError: string | null;
  batchProgress: KbBatchProgress | null;
  refreshCollections: () => Promise<void>;
  createCollection: (name: string, description: string) => Promise<void>;
  deleteCollection: (name: string) => Promise<void>;
  renameCollection: (oldName: string, newName: string) => Promise<void>;
  setSelectedCollection: (name: string | null) => void;
  loadEntries: (collection: string) => Promise<void>;
  getEntryDetail: (entryId: string, resolution?: string) => Promise<KnowledgeEntryDetail | null>;
  deleteEntry: (entryId: string) => Promise<void>;
  search: (query: string, domain?: string, limit?: number) => Promise<KnowledgeSearchResult[]>;
  ingest: (source: string, domain: string | undefined, collection: string, options?: { useLlmSummary?: boolean; extractMetadata?: boolean }) => Promise<void>;
  ingestBatch: (sources: string[], domain: string | undefined, collection: string, options?: { useLlmSummary?: boolean; extractMetadata?: boolean }) => Promise<void>;
  cancelIngest: () => void;
  clearIngestStatus: () => void;
  refreshStats: () => Promise<void>;
}

export function useKnowledge(): UseKnowledgeReturn {
  const [collections, setCollections] = useState<KnowledgeCollectionInfo[]>([]);
  const [entries, setEntries] = useState<KnowledgeEntryInfo[]>([]);
  const [selectedCollection, setSelectedCollection] = useState<string | null>(null);
  const [stats, setStats] = useState<KnowledgeStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [ingestStatus, setIngestStatus] = useState<KbIngestStatus>('idle');
  const [ingestError, setIngestError] = useState<string | null>(null);
  const [batchProgress, setBatchProgress] = useState<KbBatchProgress | null>(null);

  // -----------------------------------------------------------------------
  // Collections
  // -----------------------------------------------------------------------

  const refreshCollections = useCallback(async () => {
    setLoading(true);
    try {
      const list = await transport.invoke<KnowledgeCollectionInfo[]>('kb_collection_list');
      setCollections(list);
    } catch (err) {
      console.error('Failed to load collections:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  const createCollection = useCallback(async (name: string, description: string) => {
    try {
      await transport.invoke<KnowledgeCollectionInfo>('kb_collection_create', { name, description });
      await refreshCollections();
    } catch (err) {
      console.error('Failed to create collection:', err);
    }
  }, [refreshCollections]);

  const deleteCollection = useCallback(async (name: string) => {
    try {
      await transport.invoke('kb_collection_delete', { name });
      await refreshCollections();
      if (selectedCollection === name) {
        setSelectedCollection(null);
        setEntries([]);
      }
    } catch (err) {
      console.error('Failed to delete collection:', err);
    }
  }, [refreshCollections, selectedCollection]);

  const renameCollection = useCallback(async (oldName: string, newName: string) => {
    try {
      await transport.invoke('kb_collection_rename', { oldName, newName });
      await refreshCollections();
      // If the renamed collection is currently selected, track the new name.
      if (selectedCollection === oldName) {
        setSelectedCollection(newName);
      }
    } catch (err) {
      console.error('Failed to rename collection:', err);
      throw err; // Re-throw so UI can show error
    }
  }, [refreshCollections, selectedCollection]);

  // -----------------------------------------------------------------------
  // Stats (defined early so deleteEntry can reference it)
  // -----------------------------------------------------------------------

  const refreshStats = useCallback(async () => {
    try {
      const s = await transport.invoke<KnowledgeStats>('kb_stats');
      setStats(s);
    } catch (err) {
      console.error('Failed to load kb stats:', err);
    }
  }, []);

  // -----------------------------------------------------------------------
  // Entries
  // -----------------------------------------------------------------------

  const loadEntries = useCallback(async (collection: string) => {
    try {
      const list = await transport.invoke<KnowledgeEntryInfo[]>('kb_entry_list', { collection });
      setEntries(list);
    } catch (err) {
      console.error('Failed to load entries:', err);
      setEntries([]);
    }
  }, []);

  const getEntryDetail = useCallback(async (
    entryId: string,
    resolution: string = 'l0',
  ): Promise<KnowledgeEntryDetail | null> => {
    try {
      return await transport.invoke<KnowledgeEntryDetail>('kb_entry_detail', { entryId, resolution });
    } catch (err) {
      console.error('Failed to get entry detail:', err);
      return null;
    }
  }, []);

  const deleteEntry = useCallback(async (entryId: string) => {
    try {
      await transport.invoke('kb_entry_delete', { entryId });
      if (selectedCollection) {
        await loadEntries(selectedCollection);
      }
      // Refresh collections (entry count, space) and stats so UI updates immediately.
      await refreshCollections();
      await refreshStats();
    } catch (err) {
      console.error('Failed to delete entry:', err);
    }
  }, [loadEntries, selectedCollection, refreshCollections, refreshStats]);

  // -----------------------------------------------------------------------
  // Search
  // -----------------------------------------------------------------------

  const search = useCallback(async (
    query: string,
    domain?: string,
    limit: number = 5,
  ): Promise<KnowledgeSearchResult[]> => {
    try {
      return await transport.invoke<KnowledgeSearchResult[]>('kb_search', { query, domain, limit });
    } catch (err) {
      console.error('Failed to search kb:', err);
      return [];
    }
  }, []);

  // -----------------------------------------------------------------------
  // Ingestion
  // -----------------------------------------------------------------------

  // Cancellation ref — checked between files in ingestBatch.
  const ingestCancelledRef = useRef(false);

  const ingest = useCallback(async (
    source: string,
    domain: string | undefined,
    collection: string,
    options?: { useLlmSummary?: boolean; extractMetadata?: boolean },
  ) => {
    setIngestStatus('ingesting');
    setIngestError(null);
    ingestCancelledRef.current = false;
    try {
      const result = await transport.invoke<KnowledgeIngestResult>(
        'kb_ingest',
        buildKnowledgeIngestPayload({
          source,
          domain,
          collection,
          options,
        }),
      );
      if (ingestCancelledRef.current) {
        setIngestStatus('idle');
        return;
      }
      if (result.success) {
        setIngestStatus('success');
        await refreshCollections();
        if (selectedCollection) {
          await loadEntries(selectedCollection);
        }
      } else {
        setIngestStatus('error');
        setIngestError(result.message);
      }
    } catch (err) {
      if (!ingestCancelledRef.current) {
        setIngestStatus('error');
        setIngestError(String(err));
        console.error('Failed to ingest:', err);
      }
    }
  }, [refreshCollections, loadEntries, selectedCollection]);

  const ingestBatch = useCallback(async (
    sources: string[],
    domain: string | undefined,
    collection: string,
    options?: { useLlmSummary?: boolean; extractMetadata?: boolean },
  ) => {
    if (sources.length === 0) return;
    setIngestStatus('ingesting');
    setIngestError(null);
    setBatchProgress({ current: 0, total: sources.length });
    ingestCancelledRef.current = false;

    // Listen for progress counter updates.
    const unlistenProgress = await transport.listen<{ current: number; total: number; source: string }>(
      'kb:batch_progress',
      (event) => {
        setBatchProgress({ current: event.payload.current, total: event.payload.total });
      },
    );

    // Listen for per-file completion events. The backend includes the
    // full entry data in the event payload so we can merge it directly
    // into local state WITHOUT making additional invoke calls (which
    // would compete for the same backend mutex and block user clicks).
    const unlistenIngested = await transport.listen<{
      entry_id: string;
      source: string;
      collection: string;
      current: number;
      total: number;
      entry: KnowledgeEntryInfo | null;
    }>(
      'kb:entry_ingested',
      (event) => {
        const { entry, collection: entryCollection } = event.payload;
        if (entry && selectedCollection && entryCollection === selectedCollection) {
          // Append the new entry to the local list -- no backend call needed.
          setEntries(prev => {
            // Guard against duplicates (in case of event replay).
            if (prev.some(e => e.id === entry.id)) return prev;
            return [...prev, entry];
          });
        }
      },
    );

    try {
      const result = await transport.invoke<{ succeeded: number; failed: number; errors: string[] }>(
        'kb_ingest_batch',
        buildKnowledgeIngestBatchPayload({
          sources,
          domain,
          collection,
          options,
        }),
      );

      if (ingestCancelledRef.current) {
        setIngestStatus('idle');
        setBatchProgress(null);
      } else if (result.failed > 0) {
        setIngestStatus('error');
        setIngestError(result.errors.join('\n'));
      } else {
        setIngestStatus('success');
      }
    } catch (err) {
      if (!ingestCancelledRef.current) {
        setIngestStatus('error');
        setIngestError(String(err));
      }
    } finally {
      unlistenProgress();
      unlistenIngested();
    }

    // Final refresh to ensure authoritative state (collection stats etc.).
    await refreshCollections();
    if (selectedCollection) {
      await loadEntries(selectedCollection);
    }
  }, [refreshCollections, loadEntries, selectedCollection]);

  const cancelIngest = useCallback(() => {
    ingestCancelledRef.current = true;
    setIngestStatus('idle');
    setIngestError(null);
    setBatchProgress(null);
  }, []);

  const clearIngestStatus = useCallback(() => {
    setIngestStatus('idle');
    setIngestError(null);
    setBatchProgress(null);
  }, []);

  // (refreshStats is defined earlier, before the Entries section)

  // Auto-select collection → load entries.
  useEffect(() => {
    if (selectedCollection) {
      loadEntries(selectedCollection);
    } else {
      setEntries([]);
    }
  }, [selectedCollection, loadEntries]);

  // Load collections on mount.
  useEffect(() => {
    refreshCollections();
    refreshStats();
  }, [refreshCollections, refreshStats]);

  return {
    // State
    collections,
    entries,
    selectedCollection,
    stats,
    loading,
    ingestStatus,
    ingestError,
    batchProgress,
    // Actions
    refreshCollections,
    createCollection,
    deleteCollection,
    renameCollection,
    setSelectedCollection,
    loadEntries,
    getEntryDetail,
    deleteEntry,
    search,
    ingest,
    ingestBatch,
    cancelIngest,
    clearIngestStatus,
    refreshStats,
  };
}
