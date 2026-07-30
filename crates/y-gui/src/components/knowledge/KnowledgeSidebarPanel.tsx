import { useState, useMemo } from 'react';
import {
  X,
  Plus,
  Database,
} from 'lucide-react';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import type { KnowledgeCollectionInfo } from '../../types';
import type { KbIngestStatus, KbBatchProgress } from '../../hooks/useKnowledge';
import { useSidebarSearch } from '../../hooks/useSidebarSearch';
import { SidebarOperationStatus } from '../common/SidebarOperationStatus';
import { SidebarSearchHeader } from '../common/SidebarSearchHeader';
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
      <SidebarSearchHeader
        label="Collections"
        count={collections.length}
        searchTitle="Search collections"
        searchPlaceholder="Search collections..."
        searchOpen={searchOpen}
        searchQuery={searchQuery}
        searchInputRef={searchInputRef}
        onSearchQueryChange={setSearchQuery}
        onSearchToggle={() => {
          if (searchOpen) closeSearch();
          else setSearchOpen(true);
        }}
        actions={
          <Button
            variant="icon"
            size="sm"
            onClick={() => setShowNewCollection(true)}
            title="New Collection"
          >
            <Plus size={14} />
          </Button>
        }
      />
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

      <SidebarOperationStatus
        status={kbIngestStatus === 'ingesting' ? 'running' : kbIngestStatus}
        runningMessage={kbBatchProgress
          ? `Importing ${kbBatchProgress.current}/${kbBatchProgress.total}...`
          : 'Importing...'}
        successMessage={kbBatchProgress
          ? `${kbBatchProgress.total} file${kbBatchProgress.total > 1 ? 's' : ''} imported`
          : 'Import complete'}
        errorMessage={kbIngestError}
        fallbackErrorMessage="Import failed"
        onDismiss={onClearKbIngestStatus}
        onCancel={onCancelKbIngest}
      />
    </>
  );
}
