import { useState, useCallback } from 'react';
import { Upload, X, FolderOpen, FilePlus, Plus } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import type { KnowledgeCollectionInfo } from '../../types';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Button,
  Input,
  Badge,
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
  SelectValue,
} from '../ui';

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
          const expandedFiles = await invoke<string[]>('kb_expand_folder', { path: selected as string });
          if (expandedFiles.length > 0) {
            addFiles(expandedFiles);
          } else {
            console.warn('No supported files found in folder:', selected);
          }
        } catch (err) {
          console.error('Failed to expand folder:', err);
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
    onClose();
  };

  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent width="480px" className="text-left items-stretch">
        <DialogTitle className="text-left flex items-center gap-2">
          <Upload size={16} />
          Import Knowledge
        </DialogTitle>

        <div className="flex flex-col gap-3 mt-2">
          {/* Add buttons row */}
          <div className="flex gap-2">
            <Button variant="outline" onClick={handleAddFiles}>
              <FilePlus size={14} />
              Add Files
            </Button>
            <Button variant="outline" onClick={handleAddFolder} disabled={expanding}>
              <FolderOpen size={14} />
              {expanding ? 'Scanning...' : 'Add Folder'}
            </Button>
          </div>

          {/* Manual path input */}
          <div className="flex gap-2">
            <Input
              className="flex-1"
              placeholder="Or type a path and press Enter..."
              value={manualPath}
              onChange={e => setManualPath(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') handleManualAdd(); }}
            />
            <Button
              variant="icon"
              onClick={handleManualAdd}
              disabled={!manualPath.trim()}
              title="Add path"
            >
              <Plus size={14} />
            </Button>
          </div>

          {/* File list */}
          {files.length > 0 && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="text-11px text-[var(--text-secondary)]">
                  {files.length} file{files.length > 1 ? 's' : ''} selected
                </span>
                <button
                  className={[
                    'border-none bg-none',
                    'text-11px font-500 text-[var(--error)]',
                    'cursor-pointer font-sans',
                    'hover:underline',
                  ].join(' ')}
                  onClick={() => setFiles([])}
                >
                  Clear all
                </button>
              </div>
              <div className="flex flex-wrap gap-1 max-h-30 overflow-y-auto">
                {files.map((file, i) => {
                  const basename = file.split('/').pop() || file;
                  return (
                    <Badge
                      key={`${file}-${i}`}
                      variant="outline"
                      title={file}
                      onDismiss={() => removeFile(i)}
                    >
                      {basename}
                    </Badge>
                  );
                })}
              </div>
            </div>
          )}

          {/* Options */}
          <div className="flex flex-col gap-1">
            <label className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
              Domain (optional)
            </label>
            <Input
              placeholder="e.g. rust, python, architecture"
              value={domain}
              onChange={e => setDomain(e.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1">
            <label className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
              Collection
            </label>
            <Select value={collection} onValueChange={setCollection}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {collections.map(c => (
                  <SelectItem key={c.name} value={c.name}>{c.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="flex gap-2 justify-end mt-3">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleSubmit}
            disabled={files.length === 0}
          >
            <Upload size={14} />
            Import {files.length > 0 ? `(${files.length})` : ''}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
