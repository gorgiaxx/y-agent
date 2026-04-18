import { useState, useMemo, useCallback } from 'react';
import {
  X,
  Plus,
  Search,
  Database,
  Loader2,
  CheckCircle2,
  AlertCircle,
  ChevronRight,
  ChevronDown,
  Copy,
  Check,
} from 'lucide-react';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import type { KnowledgeCollectionInfo } from '../../types';
import type { KbIngestStatus, KbBatchProgress } from '../../hooks/useKnowledge';
import { useSidebarSearch } from '../../hooks/useSidebarSearch';
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
  const { searchQuery, setSearchQuery, searchOpen, setSearchOpen, searchInputRef, closeSearch } = useSidebarSearch();
  const [showNewCollection, setShowNewCollection] = useState(false);
  const [newCollName, setNewCollName] = useState('');
  const [newCollDesc, setNewCollDesc] = useState('');
  const [ingestStatusExpanded, setIngestStatusExpanded] = useState(false);
  const [copied, setCopied] = useState(false);

  const copyError = useCallback(() => {
    if (!kbIngestError) return;
    navigator.clipboard.writeText(kbIngestError).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [kbIngestError]);

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
      <div className="agent-session-toolbar">
        <div className="agent-session-toolbar-label">
          <span>Collections</span>
          <div className="agent-session-toolbar-meta">
            <Badge variant="outline">{collections.length}</Badge>
          </div>
        </div>
        <div className="agent-session-toolbar-actions">
          <Button
            variant="icon"
            size="sm"
            onClick={() => {
              if (searchOpen) {
                closeSearch();
              } else {
                setSearchOpen(true);
              }
            }}
            title="Search collections"
          >
            <Search size={14} />
          </Button>
          <Button
            variant="icon"
            size="sm"
            onClick={() => setShowNewCollection(true)}
            title="New Collection"
          >
            <Plus size={14} />
          </Button>
        </div>
      </div>
      {searchOpen && (
        <div className="sidebar-inline-search">
          <input
            ref={searchInputRef}
            type="text"
            placeholder="Search collections..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="sidebar-inline-search-input"
            onKeyDown={(e) => {
              if (e.key === 'Escape') closeSearch();
            }}
          />
        </div>
      )}
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
            <Button
              variant="icon"
              size="sm"
              onClick={() => setShowNewCollection(false)}
              title="Close"
              className="kb-new-collection-form-close"
            >
              <X size={12} />
            </Button>
          </div>
          <Input
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
          <Input
            placeholder="Description (optional)"
            value={newCollDesc}
            onChange={(e) => setNewCollDesc(e.target.value)}
          />
          <Button
            variant="primary"
            className="w-full font-600"
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
          </Button>
        </div>
      )}

      {/* Knowledge ingest status bar */}
      {kbIngestStatus !== 'idle' && (
        <div className={`import-status import-status--${kbIngestStatus === 'ingesting' ? 'importing' : kbIngestStatus} ${ingestStatusExpanded ? 'import-status--expanded' : ''}`}>
          <div className="import-status-row">
            {kbIngestStatus === 'ingesting' && (
              <>
                <Loader2 size={14} className="import-status-spinner" />
                <span className={`import-status-msg ${ingestStatusExpanded ? 'import-status-msg--expanded' : ''}`}>
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
                <AlertCircle size={14} className="import-status-icon" />
                <span className={`import-status-msg ${ingestStatusExpanded ? 'import-status-msg--expanded' : ''}`}>{kbIngestError || 'Import failed'}</span>
              </>
            )}
            <div className="import-status-actions">
              {kbIngestStatus === 'error' && (
                <button
                  className="import-status-copy"
                  onClick={copyError}
                  title="Copy error"
                >
                  {copied ? <Check size={12} /> : <Copy size={12} />}
                </button>
              )}
              {kbIngestStatus === 'error' && (
                <button
                  className="import-status-toggle"
                  onClick={() => setIngestStatusExpanded(!ingestStatusExpanded)}
                  title={ingestStatusExpanded ? 'Collapse' : 'Expand'}
                >
                  {ingestStatusExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </button>
              )}
              {kbIngestStatus === 'ingesting' && (
                <button className="import-status-dismiss" onClick={onCancelKbIngest} title="Cancel">
                  <X size={12} />
                </button>
              )}
              {kbIngestStatus !== 'ingesting' && (
                <button className="import-status-dismiss" onClick={() => { onClearKbIngestStatus(); setIngestStatusExpanded(false); }} title="Dismiss">
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
