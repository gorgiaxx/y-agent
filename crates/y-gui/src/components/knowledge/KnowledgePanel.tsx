import { useState, useCallback } from 'react';
import { ArrowLeft, BookOpen, Trash2, Search, Upload, ChevronDown, ChevronRight, FileText, Database, BarChart3, File, Clock, HardDrive, Globe, Plug, FolderOpen, Pencil, Check, X } from 'lucide-react';
import type { KnowledgeCollectionInfo, KnowledgeEntryInfo, KnowledgeEntryDetail, KnowledgeSearchResult } from '../../types';
import { KnowledgeIngestDialog } from './KnowledgeIngestDialog';
import { ConfirmDialog } from '../common/ConfirmDialog';
import './KnowledgePanel.css';

/** Format bytes to human-readable string. */
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val < 10 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}

/** Format a number with K/M suffix. */
function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

/** Extract filename from a full path. */
function extractFilename(uri: string): string {
  const parts = uri.split('/');
  return parts[parts.length - 1] || uri;
}

/** File type icon based on source type / extension. */
function fileTypeIcon(sourceType: string, uri: string): React.ReactNode {
  const ext = uri.split('.').pop()?.toLowerCase() || '';
  if (ext === 'md' || ext === 'markdown') return <FileText size={14} />;
  if (ext === 'pdf') return <BookOpen size={14} />;
  if (sourceType === 'web') return <Globe size={14} />;
  if (sourceType === 'api') return <Plug size={14} />;
  return <File size={14} />;
}

/** Format relative time from ISO string. */
function formatRelativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString();
}

const L2_PAGE_SIZE = 20;

interface KnowledgePanelProps {
  collections: KnowledgeCollectionInfo[];
  entries: KnowledgeEntryInfo[];
  selectedCollection: string | null;
  onSelectCollection: (name: string | null) => void;
  onCreateCollection: (name: string, description: string) => void;
  onDeleteCollection: (name: string) => Promise<void>;
  onRenameCollection: (oldName: string, newName: string) => Promise<void>;
  onGetEntryDetail: (entryId: string, resolution?: string) => Promise<KnowledgeEntryDetail | null>;
  onDeleteEntry: (entryId: string) => void;
  onSearch: (query: string, domain?: string, limit?: number) => Promise<KnowledgeSearchResult[]>;
  onIngestBatch: (sources: string[], domain: string | undefined, collection: string) => void;
}

export function KnowledgePanel({
  collections,
  entries,
  selectedCollection,
  onSelectCollection,
  onCreateCollection,
  onDeleteCollection,
  onRenameCollection,
  onGetEntryDetail,
  onDeleteEntry,
  onSearch,
  onIngestBatch,
}: KnowledgePanelProps) {
  const [showNewCollection, setShowNewCollection] = useState(false);
  const [newCollName, setNewCollName] = useState('');
  const [newCollDesc, setNewCollDesc] = useState('');
  const [showIngestDialog, setShowIngestDialog] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<KnowledgeSearchResult[]>([]);
  const [showSearch, setShowSearch] = useState(false);
  const [selectedEntry, setSelectedEntry] = useState<KnowledgeEntryDetail | null>(null);
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set());
  const [deleteTarget, setDeleteTarget] = useState<{ type: 'collection' | 'entry'; id: string; name: string } | null>(null);
  const [deletingCollection, setDeletingCollection] = useState(false);
  const [editingName, setEditingName] = useState(false);
  const [editNameValue, setEditNameValue] = useState('');
  const [l2Page, setL2Page] = useState(1);

  const handleStartRename = useCallback(() => {
    setEditNameValue(selectedCollection || '');
    setEditingName(true);
  }, [selectedCollection]);

  const handleConfirmRename = useCallback(async () => {
    const trimmed = editNameValue.trim();
    if (!trimmed || !selectedCollection || trimmed === selectedCollection) {
      setEditingName(false);
      return;
    }
    try {
      await onRenameCollection(selectedCollection, trimmed);
      onSelectCollection(trimmed);
    } catch {
      // error is logged by the hook
    }
    setEditingName(false);
  }, [editNameValue, selectedCollection, onRenameCollection, onSelectCollection]);

  const handleCancelRename = useCallback(() => {
    setEditingName(false);
  }, []);

  const handleCreateCollection = useCallback(() => {
    if (!newCollName.trim()) return;
    onCreateCollection(newCollName.trim(), newCollDesc.trim());
    setNewCollName('');
    setNewCollDesc('');
    setShowNewCollection(false);
  }, [newCollName, newCollDesc, onCreateCollection]);

  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim()) return;
    const results = await onSearch(searchQuery, undefined, 10);
    setSearchResults(results);
    setShowSearch(true);
  }, [searchQuery, onSearch]);

  const handleViewEntry = useCallback(async (entryId: string) => {
    setL2Page(1); // Reset pagination
    const detail = await onGetEntryDetail(entryId, 'l0');
    setSelectedEntry(detail);
  }, [onGetEntryDetail]);

  const toggleSection = useCallback((key: string) => {
    setExpandedSections(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const currentCollection = collections.find(c => c.name === selectedCollection);

  // Compute aggregate domain distribution from entries
  const domainCounts: Record<string, number> = {};
  entries.forEach(e => e.domains.forEach(d => {
    domainCounts[d] = (domainCounts[d] || 0) + 1;
  }));
  const topDomains = Object.entries(domainCounts)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8);

  // Average quality
  const avgQuality = entries.length > 0
    ? entries.reduce((sum, e) => sum + e.quality_score, 0) / entries.length
    : 0;

  // Total content size from entries
  const totalContentSize = entries.reduce((sum, e) => sum + (e.content_size || 0), 0);

  // ------- Empty State -------
  if (!selectedCollection && !showSearch) {
    return (
      <div className="knowledge-panel">
        <div className="knowledge-empty">
          <BookOpen size={40} className="knowledge-empty-icon" />
          <p className="knowledge-empty-title">Knowledge Base</p>
          <p className="knowledge-empty-desc">
            Select a collection from the sidebar or search across all knowledge.
          </p>
        </div>
      </div>
    );
  }

  // ------- Search View -------
  if (showSearch) {
    return (
      <div className="knowledge-panel">
        <div className="knowledge-header">
          <div className="knowledge-header-title">
            <Search size={18} />
            <h2>Search Results</h2>
          </div>
          <button className="kb-btn kb-btn--ghost" onClick={() => setShowSearch(false)}>
            Back
          </button>
        </div>
        <div className="knowledge-search-bar">
          <input
            className="kb-input"
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            placeholder="Search knowledge base..."
            onKeyDown={e => e.key === 'Enter' && handleSearch()}
          />
          <button className="kb-btn kb-btn--primary" onClick={handleSearch}>
            <Search size={14} /> Search
          </button>
        </div>
        <div className="knowledge-results">
          {searchResults.length === 0 ? (
            <div className="knowledge-results-empty">No results found</div>
          ) : (
            searchResults.map((r, i) => (
              <div key={r.chunk_id} className="knowledge-result-card">
                <div className="knowledge-result-rank">{i + 1}</div>
                <div className="knowledge-result-body">
                  <div className="knowledge-result-title">
                    {r.title}
                    <span className="knowledge-result-relevance">
                      {(r.relevance * 100).toFixed(0)}%
                    </span>
                  </div>
                  <div className="knowledge-result-domains">
                    {r.domains.map(d => (
                      <span key={d} className="kb-tag">{d}</span>
                    ))}
                  </div>
                  <p className="knowledge-result-content">
                    {r.content.slice(0, 200)}
                    {r.content.length > 200 ? '...' : ''}
                  </p>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    );
  }

  // ------- Entry Detail View -------
  if (selectedEntry) {
    return (
      <div className="knowledge-panel">
        <div className="knowledge-header">
          <div className="knowledge-header-title">
            <button
              className="kb-btn kb-btn--ghost kb-btn--icon"
              onClick={() => setSelectedEntry(null)}
              title="Back to entries"
            >
              <ArrowLeft size={18} />
            </button>
            <h2>{selectedEntry.title || 'Entry Detail'}</h2>
            <span className={`kb-badge kb-badge--${selectedEntry.state}`}>
              {selectedEntry.state}
            </span>
          </div>
          <button
            className="kb-btn kb-btn--danger"
            onClick={() => setDeleteTarget({ type: 'entry', id: selectedEntry.id, name: selectedEntry.title || 'this entry' })}
            title="Delete entry"
          >
            <Trash2 size={14} />
          </button>
        </div>

        <div className="knowledge-entry-body">
        <div className="knowledge-entry-meta">
          {selectedEntry.source_uri && (
            <span className="kb-meta-item"><FolderOpen size={13} /> {selectedEntry.source_uri}</span>
          )}
          <span className="kb-meta-item">Quality: {(selectedEntry.quality_score * 100).toFixed(0)}%</span>
          <span className="kb-meta-item">Hits: {selectedEntry.hit_count}</span>
          {selectedEntry.domains.map(d => (
            <span key={d} className="kb-tag">{d}</span>
          ))}
        </div>

        {/* L0 Summary */}
        <div className="knowledge-section">
          <div
            className="knowledge-section-header"
            onClick={() => toggleSection('l0')}
          >
            {expandedSections.has('l0') ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            <span>L0 Summary</span>
          </div>
          {(expandedSections.has('l0') || !expandedSections.has('l1')) && (
            <div className="knowledge-section-content">
              {selectedEntry.l0_summary || 'No summary available.'}
            </div>
          )}
        </div>

        {/* L1 Sections */}
        {selectedEntry.l1_sections.length > 0 && (
          <div className="knowledge-section">
            <div
              className="knowledge-section-header"
              onClick={() => toggleSection('l1')}
            >
              {expandedSections.has('l1') ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <span>L1 Section Summaries ({selectedEntry.l1_sections.length})</span>
            </div>
            {expandedSections.has('l1') && (
              <div className="knowledge-section-content">
                {selectedEntry.l1_sections.map(s => (
                  <div key={s.index} className="knowledge-l1-item">
                    <strong>§ {s.title}</strong>
                    <p>{s.summary}</p>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* L2 Chunks (paginated) */}
        {selectedEntry.l2_chunks.length > 0 && (
          <div className="knowledge-section">
            <div
              className="knowledge-section-header"
              onClick={() => toggleSection('l2')}
            >
              {expandedSections.has('l2') ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <span>L2 Full Content ({formatCount(selectedEntry.total_chunk_count || selectedEntry.l2_chunks.length)} chunks total, {selectedEntry.l2_chunks.length} loaded)</span>
            </div>
            {expandedSections.has('l2') && (
              <div className="knowledge-section-content">
                {selectedEntry.l2_chunks.slice(0, l2Page * L2_PAGE_SIZE).map(c => (
                  <div key={c.id} className="knowledge-l2-chunk">
                    <div className="knowledge-l2-chunk-header">
                      Chunk #{c.section_index + 1} · {c.token_estimate} tokens
                    </div>
                    <pre className="knowledge-l2-chunk-content">{c.content}</pre>
                  </div>
                ))}
                {l2Page * L2_PAGE_SIZE < selectedEntry.l2_chunks.length && (
                  <button
                    className="kb-btn kb-btn--primary knowledge-load-more"
                    onClick={() => setL2Page(p => p + 1)}
                  >
                    Show More ({Math.min(L2_PAGE_SIZE, selectedEntry.l2_chunks.length - l2Page * L2_PAGE_SIZE)} of {selectedEntry.l2_chunks.length - l2Page * L2_PAGE_SIZE} remaining)
                  </button>
                )}
                {selectedEntry.total_chunk_count > selectedEntry.l2_chunks.length && (
                  <div className="knowledge-chunk-note">
                    Showing {selectedEntry.l2_chunks.length} of {formatCount(selectedEntry.total_chunk_count)} total chunks. Only the first {selectedEntry.l2_chunks.length} are loaded.
                  </div>
                )}
              </div>
            )}
          </div>
        )}
        </div>

        {/* Confirm Delete Dialog */}
        <ConfirmDialog
          open={deleteTarget !== null}
          title="Delete Entry"
          message={`Are you sure you want to delete "${deleteTarget?.name ?? ''}"? This action cannot be undone.`}
          confirmLabel="Delete"
          variant="danger"
          onConfirm={() => {
            if (!deleteTarget) return;
            onDeleteEntry(deleteTarget.id);
            if (selectedEntry?.id === deleteTarget.id) {
              setSelectedEntry(null);
            }
            setDeleteTarget(null);
          }}
          onCancel={() => setDeleteTarget(null)}
        />
      </div>
    );
  }

  // ------- Collection Detail View -------
  return (
    <div className="knowledge-panel">
      <div className="knowledge-header">
        <div className="knowledge-header-title">
          <Database size={18} />
          {editingName ? (
            <div className="knowledge-rename-inline">
              <input
                className="kb-input knowledge-rename-input"
                value={editNameValue}
                onChange={e => setEditNameValue(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') handleConfirmRename();
                  if (e.key === 'Escape') handleCancelRename();
                }}
                autoFocus
              />
              <button
                className="kb-btn kb-btn--ghost kb-btn--icon"
                onClick={handleConfirmRename}
                title="Confirm rename"
              >
                <Check size={14} />
              </button>
              <button
                className="kb-btn kb-btn--ghost kb-btn--icon"
                onClick={handleCancelRename}
                title="Cancel rename"
              >
                <X size={14} />
              </button>
            </div>
          ) : (
            <>
              <h2>{currentCollection?.name || selectedCollection}</h2>
              <button
                className="kb-btn kb-btn--ghost kb-btn--icon"
                onClick={handleStartRename}
                title="Rename collection"
              >
                <Pencil size={14} />
              </button>
            </>
          )}
        </div>
        <div className="knowledge-header-actions">
          <button
            className="kb-btn kb-btn--primary"
            onClick={() => setShowIngestDialog(true)}
            title="Import document"
          >
            <Upload size={14} /> Import
          </button>
          <button
            className="kb-btn kb-btn--ghost"
            onClick={() => {
              setSearchQuery('');
              setShowSearch(true);
            }}
            title="Search"
          >
            <Search size={14} />
          </button>
          <button
            className="kb-btn kb-btn--danger"
            onClick={() => setDeleteTarget({ type: 'collection', id: selectedCollection!, name: selectedCollection! })}
            title="Delete collection"
          >
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      {currentCollection?.description && (
        <p className="knowledge-collection-desc">{currentCollection.description}</p>
      )}

      {/* Enhanced stats bar */}
      <div className="knowledge-stats-bar">
        <div className="knowledge-stat">
          <BarChart3 size={13} />
          <span>{currentCollection?.entry_count ?? 0} entries</span>
        </div>
        <div className="knowledge-stat">
          <File size={13} />
          <span>{formatCount(currentCollection?.chunk_count ?? 0)} chunks</span>
        </div>
        <div className="knowledge-stat">
          <HardDrive size={13} />
          <span>{formatBytes(totalContentSize || currentCollection?.total_bytes || 0)}</span>
        </div>
        {entries.length > 0 && (
          <div className="knowledge-stat">
            <span>Avg Quality: {(avgQuality * 100).toFixed(0)}%</span>
          </div>
        )}
        {currentCollection?.created_at && (
          <div className="knowledge-stat">
            <Clock size={13} />
            <span>{formatRelativeTime(currentCollection.created_at)}</span>
          </div>
        )}
      </div>

      {/* Domain distribution */}
      {topDomains.length > 0 && (
        <div className="knowledge-domain-bar">
          {topDomains.map(([domain, count]) => (
            <span key={domain} className="kb-tag" title={`${count} entries`}>
              {domain} ({count})
            </span>
          ))}
        </div>
      )}

      {/* Entries list */}
      <div className="knowledge-entry-list">
        {entries.length === 0 ? (
          <div className="knowledge-entry-list-empty">
            <FileText size={32} className="knowledge-empty-icon" />
            <p>No entries yet</p>
            <p className="knowledge-empty-hint">Import a document to get started.</p>
          </div>
        ) : (
          entries.map(entry => (
            <div
              key={entry.id}
              className="knowledge-entry-row"
              onClick={() => handleViewEntry(entry.id)}
            >
              <div className="knowledge-entry-main">
                <span className="knowledge-entry-type-icon">
                  {fileTypeIcon(entry.source_type || 'file', entry.source_uri)}
                </span>
                <div className="knowledge-entry-info">
                  <span className="knowledge-entry-title">{entry.title || extractFilename(entry.source_uri)}</span>
                  <span className="knowledge-entry-source">{extractFilename(entry.source_uri)}</span>
                </div>
                <div className="knowledge-entry-tags">
                  {entry.domains.map(d => (
                    <span key={d} className="kb-tag kb-tag--sm">{d}</span>
                  ))}
                </div>
              </div>
              <div className="knowledge-entry-meta-row">
                <span>{entry.chunk_count} chunks</span>
                {entry.content_size > 0 && <span>{formatBytes(entry.content_size)}</span>}
                <span>Q: {(entry.quality_score * 100).toFixed(0)}%</span>
                <span className={`kb-badge kb-badge--${entry.state} kb-badge--sm`}>
                  {entry.state}
                </span>
                <span className="knowledge-entry-time">{formatRelativeTime(entry.updated_at)}</span>
                <button
                  className="kb-btn kb-btn--danger kb-btn--sm"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteTarget({ type: 'entry', id: entry.id, name: entry.title || extractFilename(entry.source_uri) });
                  }}
                  title="Delete entry"
                >
                  <Trash2 size={12} />
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      {/* New Collection Dialog (inline) */}
      {showNewCollection && (
        <div className="knowledge-new-collection">
          <input
            className="kb-input"
            placeholder="Collection name"
            value={newCollName}
            onChange={e => setNewCollName(e.target.value)}
            autoFocus
          />
          <input
            className="kb-input"
            placeholder="Description (optional)"
            value={newCollDesc}
            onChange={e => setNewCollDesc(e.target.value)}
          />
          <div className="knowledge-new-collection-actions">
            <button className="kb-btn kb-btn--primary" onClick={handleCreateCollection}>
              Create
            </button>
            <button className="kb-btn kb-btn--ghost" onClick={() => setShowNewCollection(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Ingest Dialog */}
      {showIngestDialog && (
        <KnowledgeIngestDialog
          collections={collections}
          defaultCollection={selectedCollection || 'default'}
          onIngestBatch={onIngestBatch}
          onClose={() => setShowIngestDialog(false)}
        />
      )}

      {/* Confirm Delete Dialog */}
      <ConfirmDialog
        open={deleteTarget !== null}
        title={deleteTarget?.type === 'collection' ? 'Delete Collection' : 'Delete Entry'}
        message={
          deleteTarget?.type === 'collection'
            ? `Are you sure you want to delete the collection "${deleteTarget.name}"? All entries in this collection will be permanently removed.`
            : `Are you sure you want to delete "${deleteTarget?.name ?? ''}"? This action cannot be undone.`
        }
        confirmLabel="Delete"
        variant="danger"
        loading={deletingCollection}
        onConfirm={async () => {
          if (!deleteTarget) return;
          if (deleteTarget.type === 'collection') {
            setDeletingCollection(true);
            try {
              await onDeleteCollection(deleteTarget.id);
              if (selectedCollection === deleteTarget.id) {
                onSelectCollection(null);
              }
            } finally {
              setDeletingCollection(false);
              setDeleteTarget(null);
            }
          } else {
            onDeleteEntry(deleteTarget.id);
            setDeleteTarget(null);
          }
        }}
        onCancel={() => {
          if (!deletingCollection) setDeleteTarget(null);
        }}
      />
    </div>
  );
}
