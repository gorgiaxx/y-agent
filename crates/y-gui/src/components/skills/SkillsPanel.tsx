import { useState, useEffect, useCallback, useMemo } from 'react';
import { Puzzle, FolderOpen, Trash2, ToggleLeft, ToggleRight, ChevronRight, File, Folder, Save } from 'lucide-react';
import type { SkillDetail, SkillFileEntry } from '../../types';
import { Button, Badge } from '../ui';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { MonacoEditor } from '../ui/MonacoEditor';
import { languageFromPath } from '../ui/languageFromPath';
import './SkillsPanel.css';

interface SkillsPanelProps {
  skillName: string | null;
  onGetDetail: (name: string) => Promise<SkillDetail | null>;
  onGetFiles: (name: string) => Promise<SkillFileEntry[]>;
  onReadFile: (name: string, relativePath: string) => Promise<string | null>;
  onSaveFile: (name: string, relativePath: string, content: string) => Promise<boolean>;
  onUninstall: (name: string) => Promise<void>;
  onSetEnabled: (name: string, enabled: boolean) => Promise<void>;
  onOpenFolder: (name: string) => Promise<void>;
}

function FileTreeNode({
  entry,
  depth,
  selectedPath,
  onSelect,
  expandedDirs,
  onToggleDir,
}: {
  entry: SkillFileEntry;
  depth: number;
  selectedPath: string | null;
  onSelect: (path: string) => void;
  expandedDirs: Set<string>;
  onToggleDir: (path: string) => void;
}) {
  const isExpanded = expandedDirs.has(entry.path);

  if (entry.is_dir) {
    return (
      <div className="file-tree-dir">
        <div
          className="file-tree-item file-tree-item--dir"
          style={{ paddingLeft: `${8 + depth * 14}px` }}
          onClick={() => onToggleDir(entry.path)}
        >
          <ChevronRight
            size={12}
            className={`file-tree-chevron ${isExpanded ? 'file-tree-chevron--open' : ''}`}
          />
          <Folder size={13} className="file-tree-icon file-tree-icon--folder" />
          <span className="file-tree-name">{entry.name}</span>
        </div>
        {isExpanded && entry.children && (
          <div className="file-tree-children">
            {entry.children.map((child) => (
              <FileTreeNode
                key={child.path}
                entry={child}
                depth={depth + 1}
                selectedPath={selectedPath}
                onSelect={onSelect}
                expandedDirs={expandedDirs}
                onToggleDir={onToggleDir}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      className={`file-tree-item file-tree-item--file ${selectedPath === entry.path ? 'active' : ''}`}
      style={{ paddingLeft: `${8 + depth * 14}px` }}
      onClick={() => onSelect(entry.path)}
    >
      <File size={13} className="file-tree-icon file-tree-icon--file" />
      <span className="file-tree-name">{entry.name}</span>
    </div>
  );
}

export function SkillsPanel({
  skillName,
  onGetDetail,
  onGetFiles,
  onReadFile,
  onSaveFile,
  onUninstall,
  onSetEnabled,
  onOpenFolder,
}: SkillsPanelProps) {
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [files, setFiles] = useState<SkillFileEntry[]>([]);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState<string>('');
  const [originalContent, setOriginalContent] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const [showUninstallConfirm, setShowUninstallConfirm] = useState(false);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

  // Load skill detail and files when skill changes.
  useEffect(() => {
    if (!skillName) {
      return;
    }

    let cancelled = false;

    const load = async () => {
      const [d, f] = await Promise.all([
        onGetDetail(skillName),
        onGetFiles(skillName),
      ]);
      if (cancelled) return;
      setDetail(d);
      setFiles(f);
      setSelectedFilePath(null);
      setFileContent('');
      setOriginalContent('');
      setShowUninstallConfirm(false);
      // Expand all directories by default.
      const allDirs = new Set<string>();
      const collectDirs = (entries: SkillFileEntry[]) => {
        for (const e of entries) {
          if (e.is_dir) {
            allDirs.add(e.path);
            if (e.children) collectDirs(e.children);
          }
        }
      };
      collectDirs(f);
      setExpandedDirs(allDirs);
    };

    load();
    return () => { cancelled = true; };
  }, [skillName, onGetDetail, onGetFiles]);

  // Load file content when selected file changes.
  useEffect(() => {
    if (!skillName || !selectedFilePath) return;

    let cancelled = false;
    const loadFile = async () => {
      const content = await onReadFile(skillName, selectedFilePath);
      if (cancelled) return;
      setFileContent(content ?? '');
      setOriginalContent(content ?? '');
    };
    loadFile();
    return () => { cancelled = true; };
  }, [skillName, selectedFilePath, onReadFile]);

  const handleSave = useCallback(async () => {
    if (!skillName || !selectedFilePath) return;
    setSaving(true);
    const ok = await onSaveFile(skillName, selectedFilePath, fileContent);
    if (ok) {
      setOriginalContent(fileContent);
    }
    setSaving(false);
  }, [skillName, selectedFilePath, fileContent, onSaveFile]);

  const handleUninstall = useCallback(async () => {
    if (!skillName) return;
    await onUninstall(skillName);
    setShowUninstallConfirm(false);
  }, [skillName, onUninstall]);

  const handleToggleDir = useCallback((path: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const isDirty = fileContent !== originalContent;

  const editorLanguage = useMemo(
    () => (selectedFilePath ? languageFromPath(selectedFilePath) : 'plaintext'),
    [selectedFilePath],
  );

  // Empty state: no skill selected.
  if (!skillName) {
    return (
      <div className="skills-panel">
        <div className="skills-empty">
          <Puzzle size={40} className="skills-empty-icon" />
          <p className="skills-empty-title">Select a skill</p>
          <p className="skills-empty-desc">
            Choose a skill from the sidebar to view and edit its files.
          </p>
        </div>
      </div>
    );
  }

  // Loading state.
  if (!detail || detail.name !== skillName) {
    return (
      <div className="skills-panel">
        <div className="skills-loading">Loading skill...</div>
      </div>
    );
  }

  return (
    <div className="skills-panel">
      {/* Metadata header */}
      <div className="skill-editor-header">
        <div className="skill-editor-info">
          <div className="skill-editor-title-row">
            <Puzzle size={20} className="skill-editor-icon" />
            <h2 className="skill-editor-name">{detail.name}</h2>
            <Badge variant={detail.enabled ? 'success' : 'outline'} size="md">
              {detail.enabled ? 'Active' : 'Disabled'}
            </Badge>
          </div>
          <p className="skill-editor-desc">{detail.description}</p>
          <div className="skill-editor-meta">
            {detail.author && <span className="skill-editor-meta-item">Author: {detail.author}</span>}
            {detail.version && <span className="skill-editor-meta-item">v{detail.version}</span>}
            {detail.classification_type && <span className="skill-editor-meta-item">Type: {detail.classification_type}</span>}
          </div>
          {detail.tags.length > 0 && (
            <div className="skill-editor-tags">
              {detail.tags.map((tag) => (
                <Badge key={tag} variant="outline">{tag}</Badge>
              ))}
            </div>
          )}
        </div>
        <div className="skill-editor-actions">
          <Button
            variant="icon"
            size="sm"
            onClick={() => onOpenFolder(detail.name)}
            title="Open Folder"
          >
            <FolderOpen size={14} />
          </Button>
          <Button
            variant="icon"
            size="sm"
            onClick={async () => {
              const newEnabled = !detail.enabled;
              setDetail((prev) => prev ? { ...prev, enabled: newEnabled } : prev);
              await onSetEnabled(detail.name, newEnabled);
              // Re-fetch to confirm backend state.
              const refreshed = await onGetDetail(detail.name);
              if (refreshed) setDetail(refreshed);
            }}
            title={detail.enabled ? 'Disable' : 'Enable'}
            className={detail.enabled ? 'skill-toggle-btn skill-toggle-btn--on' : 'skill-toggle-btn'}
          >
            {detail.enabled ? <ToggleRight size={14} /> : <ToggleLeft size={14} />}
          </Button>
          <Button
            variant="icon"
            size="sm"
            onClick={() => setShowUninstallConfirm(true)}
            title="Uninstall"
            className="skill-uninstall-btn"
          >
            <Trash2 size={14} />
          </Button>
        </div>
      </div>

      {/* Editor body: file tree + text editor */}
      <div className="skill-editor-body">
        {/* File tree pane */}
        <div className="skill-editor-filetree">
          <div className="skill-editor-filetree-header">FILES</div>
          <div className="skill-editor-filetree-list">
            {files.map((entry) => (
              <FileTreeNode
                key={entry.path}
                entry={entry}
                depth={0}
                selectedPath={selectedFilePath}
                onSelect={setSelectedFilePath}
                expandedDirs={expandedDirs}
                onToggleDir={handleToggleDir}
              />
            ))}
            {files.length === 0 && (
              <div className="skill-editor-filetree-empty">No files found</div>
            )}
          </div>
        </div>

        {/* Editor pane */}
        <div className="skill-editor-pane">
          {selectedFilePath ? (
            <>
              <div className="skill-editor-pane-header">
                <span className="skill-editor-pane-filename">{selectedFilePath}</span>
                <Button
                  variant={isDirty ? 'primary' : 'ghost'}
                  size="sm"
                  onClick={handleSave}
                  disabled={!isDirty || saving}
                  title="Save (Ctrl+S)"
                >
                  <Save size={14} />
                  {saving ? 'Saving...' : 'Save'}
                </Button>
              </div>
              <MonacoEditor
                className="skill-editor-monaco"
                value={fileContent}
                onChange={(val) => setFileContent(val)}
                language={editorLanguage}
                onSave={isDirty ? handleSave : undefined}
              />
            </>
          ) : (
            <div className="skill-editor-pane-empty">
              <File size={32} className="skill-editor-pane-empty-icon" />
              <p>Select a file to edit</p>
            </div>
          )}
        </div>
      </div>

      {/* Confirm Uninstall Dialog */}
      <ConfirmDialog
        open={showUninstallConfirm}
        title="Uninstall Skill"
        message={`Are you sure you want to uninstall "${detail.name}"? This will remove all skill files and cannot be undone.`}
        confirmLabel="Uninstall"
        variant="danger"
        onConfirm={handleUninstall}
        onCancel={() => setShowUninstallConfirm(false)}
      />
    </div>
  );
}
