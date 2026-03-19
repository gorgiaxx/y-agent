import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type {
  KnowledgeCollectionInfo,
  KnowledgeEntryInfo,
  KnowledgeEntryDetail,
  KnowledgeSearchResult,
  KnowledgeIngestResult,
  KnowledgeStats,
} from '../types';

export type KbIngestStatus = 'idle' | 'ingesting' | 'success' | 'error';
export type KbBatchProgress = { current: number; total: number };

export function useKnowledge() {
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
      const list = await invoke<KnowledgeCollectionInfo[]>('kb_collection_list');
      setCollections(list);
    } catch (err) {
      console.error('Failed to load collections:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  const createCollection = useCallback(async (name: string, description: string) => {
    try {
      await invoke<KnowledgeCollectionInfo>('kb_collection_create', { name, description });
      await refreshCollections();
    } catch (err) {
      console.error('Failed to create collection:', err);
    }
  }, [refreshCollections]);

  const deleteCollection = useCallback(async (name: string) => {
    try {
      await invoke('kb_collection_delete', { name });
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
      await invoke('kb_collection_rename', { oldName, newName });
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
      const s = await invoke<KnowledgeStats>('kb_stats');
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
      const list = await invoke<KnowledgeEntryInfo[]>('kb_entry_list', { collection });
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
      return await invoke<KnowledgeEntryDetail>('kb_entry_detail', { entryId, resolution });
    } catch (err) {
      console.error('Failed to get entry detail:', err);
      return null;
    }
  }, []);

  const deleteEntry = useCallback(async (entryId: string) => {
    try {
      await invoke('kb_entry_delete', { entryId });
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
      return await invoke<KnowledgeSearchResult[]>('kb_search', { query, domain, limit });
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
  ) => {
    setIngestStatus('ingesting');
    setIngestError(null);
    ingestCancelledRef.current = false;
    try {
      const result = await invoke<KnowledgeIngestResult>('kb_ingest', {
        source,
        domain: domain || null,
        collection,
      });
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
  ) => {
    if (sources.length === 0) return;
    setIngestStatus('ingesting');
    setIngestError(null);
    setBatchProgress({ current: 0, total: sources.length });
    ingestCancelledRef.current = false;

    let hasError = false;
    const errors: string[] = [];

    for (let i = 0; i < sources.length; i++) {
      // Check cancellation before starting each file.
      if (ingestCancelledRef.current) {
        errors.push(`Cancelled after ${i} of ${sources.length} files`);
        hasError = true;
        break;
      }

      setBatchProgress({ current: i + 1, total: sources.length });
      try {
        const result = await invoke<KnowledgeIngestResult>('kb_ingest', {
          source: sources[i],
          domain: domain || null,
          collection,
        });
        if (!result.success) {
          hasError = true;
          errors.push(`${sources[i]}: ${result.message}`);
        }
      } catch (err) {
        hasError = true;
        errors.push(`${sources[i]}: ${String(err)}`);
      }
    }

    if (ingestCancelledRef.current) {
      setIngestStatus('idle');
      setBatchProgress(null);
    } else if (hasError) {
      setIngestStatus('error');
      setIngestError(errors.join('\n'));
    } else {
      setIngestStatus('success');
    }

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
