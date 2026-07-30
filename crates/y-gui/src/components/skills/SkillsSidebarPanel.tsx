import { useMemo } from 'react';
import {
  Plus,
  Puzzle,
  ShieldCheck,
} from 'lucide-react';
import { Button } from '../ui/Button';
import type { SkillInfo } from '../../types';
import type { ImportStatus } from '../../hooks/useSkills';
import { useSidebarSearch } from '../../hooks/useSidebarSearch';
import { SidebarOperationStatus } from '../common/SidebarOperationStatus';
import { SidebarSearchHeader } from '../common/SidebarSearchHeader';
interface SkillsSidebarPanelProps {
  skills: SkillInfo[];
  activeSkillName: string | null;
  importStatus: ImportStatus;
  importError: string | null;
  onSelectSkill: (name: string) => void;
  onImportClick: () => void;
  onClearImportStatus: () => void;
  onValidate?: () => void;
  validating?: boolean;
}

export function SkillsSidebarPanel({
  skills,
  activeSkillName,
  importStatus,
  importError,
  onSelectSkill,
  onImportClick,
  onClearImportStatus,
  onValidate,
  validating,
}: SkillsSidebarPanelProps) {
  const { searchQuery, setSearchQuery, searchOpen, setSearchOpen, searchInputRef, closeSearch } = useSidebarSearch();

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
      <SidebarSearchHeader
        label="Skills"
        count={skills.length}
        searchTitle="Search skills"
        searchPlaceholder="Search skills..."
        searchOpen={searchOpen}
        searchQuery={searchQuery}
        searchInputRef={searchInputRef}
        onSearchQueryChange={setSearchQuery}
        onSearchToggle={() => {
          if (searchOpen) closeSearch();
          else setSearchOpen(true);
        }}
        actions={
          <>
            {onValidate && (
              <Button
                variant="icon"
                size="sm"
                onClick={onValidate}
                disabled={validating}
                title="Validate All Skills"
              >
                <ShieldCheck size={14} className={validating ? 'spin' : ''} />
              </Button>
            )}
            <Button variant="icon" size="sm" onClick={onImportClick} title="Import Skill">
              <Plus size={14} />
            </Button>
          </>
        }
      />
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

      <SidebarOperationStatus
        status={importStatus === 'importing' ? 'running' : importStatus}
        runningMessage="Importing skill..."
        successMessage="Skill imported"
        errorMessage={importError}
        fallbackErrorMessage="Import failed"
        onDismiss={onClearImportStatus}
      />
    </>
  );
}
