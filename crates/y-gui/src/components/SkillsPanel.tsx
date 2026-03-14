import { useState } from 'react';
import { Puzzle } from 'lucide-react';
import type { SkillInfo, SkillDetail } from '../types';
import { SkillDetailDialog } from './SkillDetailDialog';
import './SkillsPanel.css';

interface SkillsPanelProps {
  skills: SkillInfo[];
  loading: boolean;
  onGetDetail: (name: string) => Promise<SkillDetail | null>;
  onUninstall: (name: string) => Promise<void>;
  onSetEnabled: (name: string, enabled: boolean) => Promise<void>;
  onOpenFolder: (name: string) => Promise<void>;
}

export function SkillsPanel({
  skills,
  loading,
  onGetDetail,
  onUninstall,
  onSetEnabled,
  onOpenFolder,
}: SkillsPanelProps) {
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const handleSkillClick = async (name: string) => {
    const detail = await onGetDetail(name);
    if (detail) {
      setSelectedSkill(detail);
      setDialogOpen(true);
    }
  };

  const handleUninstall = async (name: string) => {
    await onUninstall(name);
    setDialogOpen(false);
    setSelectedSkill(null);
  };

  const handleSetEnabled = async (name: string, enabled: boolean) => {
    await onSetEnabled(name, enabled);
    // Refresh the detail
    const detail = await onGetDetail(name);
    if (detail) {
      setSelectedSkill(detail);
    }
  };

  if (loading) {
    return (
      <div className="skills-panel">
        <div className="skills-loading">Loading skills...</div>
      </div>
    );
  }

  return (
    <div className="skills-panel">
      <div className="skills-header-bar">
        <span className="skills-count">{skills.length} skill{skills.length !== 1 ? 's' : ''} installed</span>
      </div>

      {skills.length === 0 ? (
        <div className="skills-empty">
          <Puzzle size={40} className="skills-empty-icon" />
          <p className="skills-empty-title">No skills installed</p>
          <p className="skills-empty-desc">
            Install skills by placing them in the skills directory.
          </p>
        </div>
      ) : (
        <div className="skills-grid">
          {skills.map((skill) => (
            <div
              key={skill.name}
              className={`skill-card ${!skill.enabled ? 'skill-card--disabled' : ''}`}
              onClick={() => handleSkillClick(skill.name)}
            >
              <div className="skill-card-header">
                <Puzzle size={16} className="skill-card-icon" />
                <span className="skill-card-name">{skill.name}</span>
                <span className={`skill-card-badge ${skill.enabled ? 'skill-card-badge--enabled' : 'skill-card-badge--disabled'}`}>
                  {skill.enabled ? 'Active' : 'Disabled'}
                </span>
              </div>
              <p className="skill-card-desc">{skill.description}</p>
              {skill.tags.length > 0 && (
                <div className="skill-card-tags">
                  {skill.tags.map((tag) => (
                    <span key={tag} className="skill-card-tag">{tag}</span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {dialogOpen && selectedSkill && (
        <SkillDetailDialog
          skill={selectedSkill}
          onClose={() => { setDialogOpen(false); setSelectedSkill(null); }}
          onUninstall={handleUninstall}
          onSetEnabled={handleSetEnabled}
          onOpenFolder={onOpenFolder}
        />
      )}
    </div>
  );
}
