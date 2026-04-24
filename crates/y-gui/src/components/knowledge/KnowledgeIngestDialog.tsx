import { useState, useCallback } from 'react';
import { Upload, FolderOpen, FilePlus } from 'lucide-react';
import { transport, platform } from '../../lib';
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
  onIngestBatch: (
    sources: string[],
    domain: string | undefined,
    collection: string,
    options?: { useLlmSummary?: boolean; extractMetadata?: boolean },
  ) => void;
  onClose: () => void;
}

export function KnowledgeIngestDialog({
  collections,
  defaultCollection,
  onIngestBatch,
  onClose,
}: KnowledgeIngestDialogProps) {
  const [files, setFiles] = useState<string[]>([]);
  const [domain, setDomain] = useState('');
  const [collection, setCollection] = useState(defaultCollection);
  const [manualPath, setManualPath] = useState('');
  const [expanding, setExpanding] = useState(false);
  const [useLlmSummary, setUseLlmSummary] = useState(false);
  const [extractMetadata, setExtractMetadata] = useState(false);
  const supportsNativePaths = platform.capabilities.nativeFilePaths;

  const addFiles = useCallback((paths: string[]) => {
    setFiles(prev => {
      const existing = new Set(prev);
      const newPaths = paths.filter(p => !existing.has(p));
      return [...prev, ...newPaths];
    });
  }, []);

  const addManualPath = useCallback(() => {
    const trimmed = manualPath.trim();
    if (!trimmed) return;
    addFiles([trimmed]);
    setManualPath('');
  }, [addFiles, manualPath]);

  const removeFile = useCallback((index: number) => {
    setFiles(prev => prev.filter((_, i) => i !== index));
  }, []);

  const handleAddFiles = async () => {
    try {
      const selected = await platform.openFileDialog({
        multiple: true,
        filters: [
          { name: 'Documents', extensions: ['md', 'txt', 'rst', 'json', 'yaml', 'yml', 'toml', 'csv'] },
          { name: 'All files', extensions: ['*'] },
        ],
      });
      if (selected) {
        addFiles(selected);
      }
    } catch (err) {
      console.error('Failed to open file dialog:', err);
    }
  };

  const expandFolderPath = useCallback(async (folderPath: string) => {
    setExpanding(true);
    try {
      const expandedFiles = await transport.invoke<string[]>('kb_expand_folder', { path: folderPath });
      if (expandedFiles.length > 0) {
        addFiles(expandedFiles);
      } else {
        console.warn('No supported files found in folder:', folderPath);
      }
    } catch (err) {
      console.error('Failed to expand folder:', err);
      addFiles([folderPath]);
    } finally {
      setExpanding(false);
    }
  }, [addFiles]);

  const handleAddFolder = async () => {
    try {
      const selected = await platform.openFileDialog({ directory: true });
      if (selected && selected.length > 0) {
        await expandFolderPath(selected[0]);
      }
    } catch (err) {
      console.error('Failed to open folder dialog:', err);
    }
  };

  const handleExpandManualFolder = async () => {
    const trimmed = manualPath.trim();
    if (!trimmed) return;
    await expandFolderPath(trimmed);
    setManualPath('');
  };

  const handleSubmit = () => {
    if (files.length === 0) return;
    onIngestBatch(files, domain.trim() || undefined, collection, {
      useLlmSummary,
      extractMetadata,
    });
    onClose();
  };

  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent size="md" className="text-left items-stretch">
        <DialogTitle className="text-left flex items-center gap-2">
          <Upload size={16} />
          Import Knowledge
        </DialogTitle>

        <div className="flex flex-col gap-3 mt-2">
          {/* Unified Add actions */}
          {supportsNativePaths ? (
            <div className="flex border border-solid border-[var(--border)] rounded-[var(--radius-md)] overflow-hidden w-max bg-[var(--surface-primary)]">
              <button
                onClick={handleAddFiles}
                className="flex items-center gap-1.5 px-3 h-8 text-12px font-500 font-sans text-[var(--text-secondary)] bg-transparent hover:(bg-[var(--surface-hover)] text-[var(--text-primary)]) transition-all cursor-pointer border-none outline-none m-0"
              >
                <FilePlus size={14} />
                Add Files
              </button>
              <div className="w-[1px] bg-[var(--border)] my-1.5 mx-[-1px] z-10" />
              <button
                onClick={handleAddFolder}
                disabled={expanding}
                className="flex items-center gap-1.5 px-3 h-8 text-12px font-500 font-sans text-[var(--text-secondary)] bg-transparent hover:(bg-[var(--surface-hover)] text-[var(--text-primary)]) transition-all cursor-pointer border-none outline-none m-0 disabled:(op-50 cursor-not-allowed pointer-events-none)"
              >
                <FolderOpen size={14} />
                {expanding ? 'Scanning...' : 'Add Folder'}
              </button>
            </div>
          ) : (
            <div className="flex gap-2 items-center">
              <Input
                value={manualPath}
                onChange={(event) => setManualPath(event.target.value)}
                placeholder="/srv/docs/file.md"
                variant="mono"
                className="flex-1"
              />
              <Button variant="outline" onClick={addManualPath} disabled={!manualPath.trim()}>
                <FilePlus size={14} />
                Add
              </Button>
              <Button
                variant="outline"
                onClick={handleExpandManualFolder}
                disabled={!manualPath.trim() || expanding}
              >
                <FolderOpen size={14} />
                {expanding ? 'Scanning...' : 'Scan'}
              </Button>
            </div>
          )}

          {/* File list */}
          {files.length > 0 && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="text-11px text-[var(--text-secondary)]">
                  {files.length} file{files.length > 1 ? 's' : ''} selected
                </span>
                <button
                  className={[
                    'border-transparent bg-transparent',
                    'px-1.5 py-0.5 rounded-[var(--radius-sm)]',
                    'text-11px font-500 text-[var(--text-secondary)]',
                    'cursor-pointer font-sans transition-all duration-150',
                    'hover:(text-[var(--error)] bg-[var(--error-subtle)])',
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

          {/* LLM Processing Options */}
          <div className="flex flex-col gap-2 pt-2 border-t border-[var(--border)]">
            <span className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
              AI Processing
            </span>
            <label
              className="flex items-center gap-2 cursor-pointer select-none"
              htmlFor="toggle-llm-summary"
            >
              <input
                id="toggle-llm-summary"
                type="checkbox"
                className="accent-[var(--accent)] w-3.5 h-3.5"
                checked={useLlmSummary}
                onChange={e => setUseLlmSummary(e.target.checked)}
              />
              <span className="text-12px text-[var(--text-primary)]">
                LLM Summarization
              </span>
              <span className="text-10px text-[var(--text-muted)] ml-auto">
                AI-generated L0/L1 summaries
              </span>
            </label>
            <label
              className="flex items-center gap-2 cursor-pointer select-none"
              htmlFor="toggle-extract-metadata"
            >
              <input
                id="toggle-extract-metadata"
                type="checkbox"
                className="accent-[var(--accent)] w-3.5 h-3.5"
                checked={extractMetadata}
                onChange={e => setExtractMetadata(e.target.checked)}
              />
              <span className="text-12px text-[var(--text-primary)]">
                Metadata Extraction
              </span>
              <span className="text-10px text-[var(--text-muted)] ml-auto">
                Document type, industry, sub-category
              </span>
            </label>
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
