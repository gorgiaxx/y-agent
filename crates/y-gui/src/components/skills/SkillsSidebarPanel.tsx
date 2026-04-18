import { useState, useMemo, useCallback } from 'react';
import {
  X,
  Plus,
  Search,
  Puzzle,
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
import type { SkillInfo } from '../../types';
import type { ImportStatus } from '../../hooks/useSkills';
import { useSidebarSearch } from '../../hooks/useSidebarSearch';

interface SkillsSidebarPanelProps {
  skills: SkillInfo[];
  activeSkillName: string | null;
  importStatus: ImportStatus;
  importError: string | null;
  onSelectSkill: (name: string) => void;
  onImportClick: () => void;
  onClearImportStatus: () => void;
}

export function SkillsSidebarPanel({
  skills,
  activeSkillName,
  importStatus,
  importError,
  onSelectSkill,
  onImportClick,
  onClearImportStatus,
}: SkillsSidebarPanelProps) {
  const { searchQuery, setSearchQuery, searchOpen, setSearchOpen, searchInputRef, closeSearch } = useSidebarSearch();
  const [importStatusExpanded, setImportStatusExpanded] = useState(false);
  const [copied, setCopied] = useState(false);

  const copyError = useCallback(() => {
    if (!importError) return;
    navigator.clipboard.writeText(importError).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [importError]);

  const filtered = useMemo(() => {
    if (!searchQuery) return skills;
    const q = searchQuery.toLowerCase();
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q)) ||
        s.description.toLowerCase().includes(q),
    );
  }, [skills, searchQuery]);

  return (
    <>
      <div className="agent-session-toolbar">
        <div className="agent-session-toolbar-label">
          <span>Skills</span>
          <div className="agent-session-toolbar-meta">
            <Badge variant="outline">{skills.length}</Badge>
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
            title="Search skills"
          >
            <Search size={14} />
          </Button>
          <Button variant="icon" size="sm" onClick={onImportClick} title="Import Skill">
            <Plus size={14} />
          </Button>
        </div>
      </div>
      {searchOpen && (
        <div className="sidebar-inline-search">
          <input
            ref={searchInputRef}
            type="text"
            placeholder="Search skills..."
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
            {searchQuery ? 'No matching skills' : 'No skills installed'}
          </div>
        ) : (
          filtered.map((skill) => (
            <div
              key={skill.name}
              className={`sidebar-item ${activeSkillName === skill.name ? 'active' : ''} ${!skill.enabled ? 'sidebar-item--disabled' : ''}`}
              onClick={() => onSelectSkill(skill.name)}
            >
              <div className="sidebar-item-header">
                <Puzzle size={14} className="sidebar-item-icon" />
                <span className="sidebar-item-name">{skill.name}</span>
                {!skill.enabled && (
                  <span className="sidebar-item-badge">OFF</span>
                )}
              </div>
              <p className="sidebar-item-desc">{skill.description}</p>
              {skill.tags.length > 0 && (
                <div className="sidebar-item-tags">
                  {skill.tags.slice(0, 3).map((tag) => (
                    <span key={tag} className="sidebar-item-tag">{tag}</span>
                  ))}
                </div>
              )}
            </div>
          ))
        )}
      </div>

      {/* Import status bar */}
      {importStatus !== 'idle' && (
        <div className={`import-status import-status--${importStatus} ${importStatusExpanded ? 'import-status--expanded' : ''}`}>
          <div className="import-status-row">
            {importStatus === 'importing' && (
              <>
                <Loader2 size={14} className="import-status-spinner" />
                <span className={`import-status-msg ${importStatusExpanded ? 'import-status-msg--expanded' : ''}`}>Importing skill…</span>
              </>
            )}
            {importStatus === 'success' && (
              <>
                <CheckCircle2 size={14} />
                <span className="import-status-msg">Skill imported</span>
              </>
            )}
            {importStatus === 'error' && (
              <>
                <AlertCircle size={14} className="import-status-icon" />
                <span className={`import-status-msg ${importStatusExpanded ? 'import-status-msg--expanded' : ''}`}>{importError || 'Import failed'}</span>
              </>
            )}
            <div className="import-status-actions">
              {importStatus === 'error' && (
                <button
                  className="import-status-copy"
                  onClick={copyError}
                  title="Copy error"
                >
                  {copied ? <Check size={12} /> : <Copy size={12} />}
                </button>
              )}
              {importStatus === 'error' && (
                <button
                  className="import-status-toggle"
                  onClick={() => setImportStatusExpanded(!importStatusExpanded)}
                  title={importStatusExpanded ? 'Collapse' : 'Expand'}
                >
                  {importStatusExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </button>
              )}
              {importStatus === 'error' && (
                <button className="import-status-dismiss" onClick={() => { onClearImportStatus(); setImportStatusExpanded(false); }} title="Dismiss">
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
