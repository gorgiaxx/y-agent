import { useState, useCallback } from 'react';
import { Upload, X, FolderOpen, FilePlus, Plus } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import type { KnowledgeCollectionInfo } from '../types';
import './KnowledgeIngestDialog.css';

interface KnowledgeIngestDialogProps {
  collections: KnowledgeCollectionInfo[];
  defaultCollection: string;
  onIngestBatch: (sources: string[], domain: string | undefined, collection: string) => void;
  onClose: () => void;
}

export function KnowledgeIngestDialog({
  collections,
  defaultCollection,
  onIngestBatch,
  onClose,
}: KnowledgeIngestDialogProps) {
  const [files, setFiles] = useState<string[]>([]);
  const [manualPath, setManualPath] = useState('');
  const [domain, setDomain] = useState('');
  const [collection, setCollection] = useState(defaultCollection);
  const [expanding, setExpanding] = useState(false);

  const addFiles = useCallback((paths: string[]) => {
    setFiles(prev => {
      const existing = new Set(prev);
      const newPaths = paths.filter(p => !existing.has(p));
      return [...prev, ...newPaths];
    });
  }, []);

  const removeFile = useCallback((index: number) => {
    setFiles(prev => prev.filter((_, i) => i !== index));
  }, []);

  const handleAddFiles = async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: true,
        title: 'Select files to import',
        filters: [
          { name: 'Documents', extensions: ['md', 'txt', 'rst', 'json', 'yaml', 'yml', 'toml', 'csv'] },
          { name: 'All files', extensions: ['*'] },
        ],
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        addFiles(paths);
      }
    } catch (err) {
      console.error('Failed to open file dialog:', err);
    }
  };

  const handleAddFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Select folder to import',
      });
      if (selected) {
        setExpanding(true);
        try {
          // Expand folder into individual supported files via backend
          const expandedFiles = await invoke<string[]>('kb_expand_folder', { path: selected as string });
          if (expandedFiles.length > 0) {
            addFiles(expandedFiles);
          } else {
            console.warn('No supported files found in folder:', selected);
          }
        } catch (err) {
          console.error('Failed to expand folder:', err);
          // Fallback: add the folder path directly (will likely fail on ingest
          // but at least the user sees what was selected)
          addFiles([selected as string]);
        } finally {
          setExpanding(false);
        }
      }
    } catch (err) {
      console.error('Failed to open folder dialog:', err);
    }
  };

  const handleManualAdd = () => {
    const trimmed = manualPath.trim();
    if (!trimmed) return;
    addFiles([trimmed]);
    setManualPath('');
  };

  const handleSubmit = () => {
    if (files.length === 0) return;
    onIngestBatch(files, domain.trim() || undefined, collection);
    onClose(); // Close immediately after triggering import
  };

  return (
    <div className="kb-dialog-overlay" onClick={onClose}>
      <div className="kb-dialog" onClick={e => e.stopPropagation()}>
        <div className="kb-dialog-header">
          <h3>
            <Upload size={16} />
            Import Knowledge
          </h3>
          <button className="kb-dialog-close" onClick={onClose}>
            <X size={16} />
          </button>
        </div>

        <div className="kb-dialog-body">
          {/* Add buttons row */}
          <div className="kb-add-buttons">
            <button className="kb-btn kb-btn--outline" onClick={handleAddFiles}>
              <FilePlus size={14} />
              Add Files
            </button>
            <button className="kb-btn kb-btn--outline" onClick={handleAddFolder} disabled={expanding}>
              <FolderOpen size={14} />
              {expanding ? 'Scanning...' : 'Add Folder'}
            </button>
          </div>

          {/* Manual path input */}
          <div className="kb-manual-input">
            <input
              className="kb-input"
              placeholder="Or type a path and press Enter..."
              value={manualPath}
              onChange={e => setManualPath(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') handleManualAdd(); }}
            />
            <button
              className="kb-btn kb-btn--icon"
              onClick={handleManualAdd}
              disabled={!manualPath.trim()}
              title="Add path"
            >
              <Plus size={14} />
            </button>
          </div>

          {/* File list */}
          {files.length > 0 && (
            <div className="kb-file-list">
              <div className="kb-file-list-header">
                <span>{files.length} file{files.length > 1 ? 's' : ''} selected</span>
                <button
                  className="kb-file-clear-btn"
                  onClick={() => setFiles([])}
                >
                  Clear all
                </button>
              </div>
              <div className="kb-file-tags">
                {files.map((file, i) => {
                  const basename = file.split('/').pop() || file;
                  return (
                    <div key={`${file}-${i}`} className="kb-file-tag" title={file}>
                      <span className="kb-file-tag-name">{basename}</span>
                      <button
                        className="kb-file-tag-remove"
                        onClick={() => removeFile(i)}
                        title="Remove"
                      >
                        <X size={10} />
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Options */}
          <label className="kb-dialog-label">
            Domain (optional)
            <input
              className="kb-input"
              placeholder="e.g. rust, python, architecture"
              value={domain}
              onChange={e => setDomain(e.target.value)}
            />
          </label>

          <label className="kb-dialog-label">
            Collection
            <select
              className="kb-input kb-select"
              value={collection}
              onChange={e => setCollection(e.target.value)}
            >
              {collections.map(c => (
                <option key={c.name} value={c.name}>{c.name}</option>
              ))}
            </select>
          </label>
        </div>

        <div className="kb-dialog-footer">
          <button className="kb-btn kb-btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            className="kb-btn kb-btn--primary"
            onClick={handleSubmit}
            disabled={files.length === 0}
          >
            <Upload size={14} />
            Import {files.length > 0 ? `(${files.length})` : ''}
          </button>
        </div>
      </div>
    </div>
  );
}
