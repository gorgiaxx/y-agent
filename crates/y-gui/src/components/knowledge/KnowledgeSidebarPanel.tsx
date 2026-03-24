import { useState, useMemo } from 'react';
import {
  X,
  Plus,
  Database,
  Loader2,
  CheckCircle2,
  AlertCircle,
} from 'lucide-react';
import type { KnowledgeCollectionInfo } from '../../types';
import type { KbIngestStatus, KbBatchProgress } from '../../hooks/useKnowledge';
import './KnowledgeSidebarPanel.css';

interface KnowledgeSidebarPanelProps {
  collections: KnowledgeCollectionInfo[];
  selectedCollection: string | null;
  onSelectCollection: (name: string) => void;
  onCreateCollection: (name: string, description: string) => void;
  kbIngestStatus: KbIngestStatus;
  kbBatchProgress: KbBatchProgress | null;
  kbIngestError: string | null;
  onClearKbIngestStatus: () => void;
  onCancelKbIngest: () => void;
}

export function KnowledgeSidebarPanel({
  collections,
  selectedCollection,
  onSelectCollection,
  onCreateCollection,
  kbIngestStatus,
  kbBatchProgress,
  kbIngestError,
  onClearKbIngestStatus,
  onCancelKbIngest,
}: KnowledgeSidebarPanelProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [showNewCollection, setShowNewCollection] = useState(false);
  const [newCollName, setNewCollName] = useState('');
  const [newCollDesc, setNewCollDesc] = useState('');

  const filtered = useMemo(() => {
    if (!searchQuery) return collections;
    const q = searchQuery.toLowerCase();
    return collections.filter(
      (c) =>
        c.name.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q),
    );
  }, [collections, searchQuery]);

  return (
    <>
      <div className="sidebar-header">
        <div className="sidebar-search">
          <input
            type="text"
            placeholder="Search collections..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="search-input"
          />
        </div>
        <div className="sidebar-header-actions">
          <button
            className="btn-new-chat"
            onClick={() => setShowNewCollection(true)}
            title="New Collection"
          >
            <Plus size={16} />
          </button>
        </div>
      </div>
      <div className="sidebar-list">
        {filtered.length === 0 ? (
          <div className="session-empty">
            {searchQuery ? 'No matching collections' : 'No collections yet'}
          </div>
        ) : (
          filtered.map((coll) => {
            const sizeLabel = coll.total_bytes > 0
              ? coll.total_bytes >= 1048576
                ? `${(coll.total_bytes / 1048576).toFixed(1)} MB`
                : coll.total_bytes >= 1024
                  ? `${(coll.total_bytes / 1024).toFixed(1)} KB`
                  : `${coll.total_bytes} B`
              : null;
            const chunkLabel = coll.chunk_count >= 1000
              ? `${(coll.chunk_count / 1000).toFixed(1)}K`
              : String(coll.chunk_count);
            return (
              <div
                key={coll.name}
                className={`sidebar-item ${selectedCollection === coll.name ? 'active' : ''}`}
                onClick={() => onSelectCollection(coll.name)}
              >
                <div className="sidebar-item-header">
                  <Database size={14} className="sidebar-item-icon" />
                  <span className="sidebar-item-name">{coll.name}</span>
                  <span className="sidebar-item-badge">{coll.entry_count}</span>
                </div>
                <p className="sidebar-item-desc">
                  {coll.entry_count > 0
                    ? `${coll.entry_count} entries · ${chunkLabel} chunks${sizeLabel ? ` · ${sizeLabel}` : ''}`
                    : coll.description || 'No description'}
                </p>
              </div>
            );
          })
        )}
      </div>

      {/* Inline new collection form */}
      {showNewCollection && (
        <div className="kb-new-collection-form">
          <div className="kb-new-collection-form-header">
            <span className="kb-new-collection-form-title">New Collection</span>
            <button
              className="kb-new-collection-form-close"
              onClick={() => setShowNewCollection(false)}
              title="Close"
            >
              <X size={12} />
            </button>
          </div>
          <input
            className="search-input"
            placeholder="Collection name"
            value={newCollName}
            onChange={(e) => setNewCollName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && newCollName.trim()) {
                onCreateCollection(newCollName.trim(), newCollDesc.trim());
                setNewCollName('');
                setNewCollDesc('');
                setShowNewCollection(false);
              }
            }}
            autoFocus
          />
          <input
            className="search-input"
            placeholder="Description (optional)"
            value={newCollDesc}
            onChange={(e) => setNewCollDesc(e.target.value)}
          />
          <button
            className="kb-new-collection-create-btn"
            disabled={!newCollName.trim()}
            onClick={() => {
              if (newCollName.trim()) {
                onCreateCollection(newCollName.trim(), newCollDesc.trim());
                setNewCollName('');
                setNewCollDesc('');
                setShowNewCollection(false);
              }
            }}
          >
            Create
          </button>
        </div>
      )}

      {/* Knowledge ingest status bar */}
      {kbIngestStatus !== 'idle' && (
        <div className={`import-status import-status--${kbIngestStatus === 'ingesting' ? 'importing' : kbIngestStatus}`}>
          <div className="import-status-row">
            {kbIngestStatus === 'ingesting' && (
              <>
                <Loader2 size={14} className="import-status-spinner" />
                <span className="import-status-msg">
                  {kbBatchProgress
                    ? `Importing ${kbBatchProgress.current}/${kbBatchProgress.total}…`
                    : 'Importing…'}
                </span>
              </>
            )}
            {kbIngestStatus === 'success' && (
              <>
                <CheckCircle2 size={14} />
                <span className="import-status-msg">
                  {kbBatchProgress
                    ? `${kbBatchProgress.total} file${kbBatchProgress.total > 1 ? 's' : ''} imported`
                    : 'Import complete'}
                </span>
              </>
            )}
            {kbIngestStatus === 'error' && (
              <>
                <AlertCircle size={14} />
                <span className="import-status-msg">{kbIngestError || 'Import failed'}</span>
              </>
            )}
            <div className="import-status-actions">
              {kbIngestStatus === 'ingesting' && (
                <button className="import-status-dismiss" onClick={onCancelKbIngest} title="Cancel">
                  <X size={12} />
                </button>
              )}
              {kbIngestStatus !== 'ingesting' && (
                <button className="import-status-dismiss" onClick={onClearKbIngestStatus} title="Dismiss">
                  <X size={12} />
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </>
  );
}
