import { useState } from 'react';
import { X, FolderOpen, Trash2, ToggleLeft, ToggleRight } from 'lucide-react';
import type { SkillDetail } from '../types';
import './SkillDetailDialog.css';

interface SkillDetailDialogProps {
  skill: SkillDetail;
  onClose: () => void;
  onUninstall: (name: string) => Promise<void>;
  onSetEnabled: (name: string, enabled: boolean) => Promise<void>;
  onOpenFolder: (name: string) => Promise<void>;
}

export function SkillDetailDialog({
  skill,
  onClose,
  onUninstall,
  onSetEnabled,
  onOpenFolder,
}: SkillDetailDialogProps) {
  const [confirming, setConfirming] = useState(false);

  const handleUninstall = async () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    await onUninstall(skill.name);
  };

  return (
    <div className="skill-dialog-overlay" onClick={onClose}>
      <div className="skill-dialog" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="skill-dialog-header">
          <div className="skill-dialog-title-row">
            <h2 className="skill-dialog-title">{skill.name}</h2>
            <span className={`skill-dialog-badge ${skill.enabled ? 'skill-dialog-badge--enabled' : 'skill-dialog-badge--disabled'}`}>
              {skill.enabled ? 'Active' : 'Disabled'}
            </span>
          </div>
          <button className="skill-dialog-close" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </div>

        {/* Meta */}
        <div className="skill-dialog-meta">
          {skill.author && <span className="skill-dialog-meta-item">Author: {skill.author}</span>}
          {skill.version && <span className="skill-dialog-meta-item">v{skill.version}</span>}
          {skill.classification_type && (
            <span className="skill-dialog-meta-item">Type: {skill.classification_type}</span>
          )}
        </div>

        {/* Description */}
        <p className="skill-dialog-desc">{skill.description}</p>

        {/* Tags */}
        {skill.tags.length > 0 && (
          <div className="skill-dialog-tags">
            {skill.tags.map((tag) => (
              <span key={tag} className="skill-dialog-tag">{tag}</span>
            ))}
          </div>
        )}

        {/* Root content preview */}
        <div className="skill-dialog-content-section">
          <label className="skill-dialog-content-label">Skill Instructions</label>
          <div className="skill-dialog-content-box">
            <pre className="skill-dialog-content-pre">{skill.root_content || '(no content)'}</pre>
          </div>
        </div>

        {/* Actions */}
        <div className="skill-dialog-actions">
          <button
            className="skill-dialog-btn skill-dialog-btn--folder"
            onClick={() => onOpenFolder(skill.name)}
          >
            <FolderOpen size={14} />
            Open Folder
          </button>

          <div className="skill-dialog-actions-right">
            <button
              className={`skill-dialog-btn ${skill.enabled ? 'skill-dialog-btn--disable' : 'skill-dialog-btn--enable'}`}
              onClick={() => onSetEnabled(skill.name, !skill.enabled)}
            >
              {skill.enabled ? <ToggleRight size={14} /> : <ToggleLeft size={14} />}
              {skill.enabled ? 'Disable' : 'Enable'}
            </button>

            <button
              className={`skill-dialog-btn skill-dialog-btn--uninstall ${confirming ? 'skill-dialog-btn--confirm' : ''}`}
              onClick={handleUninstall}
              onBlur={() => setConfirming(false)}
            >
              <Trash2 size={14} />
              {confirming ? 'Confirm Uninstall' : 'Uninstall'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
