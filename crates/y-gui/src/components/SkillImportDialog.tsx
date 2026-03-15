import { useState } from 'react';
import { X, Upload, Shield, FileText } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import './SkillImportDialog.css';

interface SkillImportDialogProps {
  onImport: (path: string, sanitize: boolean) => void;
  onClose: () => void;
}

export function SkillImportDialog({ onImport, onClose }: SkillImportDialogProps) {
  const [path, setPath] = useState('');
  const [sanitize, setSanitize] = useState(true);

  const handleBrowse = async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        title: 'Select Skill main file',
      });
      if (selected) {
        setPath(selected as string);
      }
    } catch (err) {
      console.error('Failed to open dialog:', err);
    }
  };

  const handleImport = () => {
    if (!path) return;
    onImport(path, sanitize);
    onClose();
  };

  return (
    <div className="import-dialog-overlay" onClick={onClose}>
      <div className="import-dialog" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="import-dialog-header">
          <h2 className="import-dialog-title">Import Skill</h2>
          <button className="import-dialog-close" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </div>

        {/* Path selection */}
        <div className="import-dialog-field">
          <label className="import-dialog-label">Source Path</label>
          <div className="import-dialog-path-row">
            <input
              type="text"
              className="import-dialog-path-input"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="Select Skill main file..."
              readOnly
            />
            <button className="import-dialog-browse" onClick={handleBrowse} title="Browse">
              <FileText size={14} />
              Browse
            </button>
          </div>
          <p className="import-dialog-hint">Please select the Skill main file</p>
        </div>

        {/* Sanitize toggle */}
        <div className="import-dialog-field">
          <label className="import-dialog-toggle-row">
            <div className="import-dialog-toggle-info">
              <Shield size={14} className="import-dialog-toggle-icon" />
              <span className="import-dialog-toggle-label">Security Check (Agent-assisted)</span>
            </div>
            <button
              className={`import-dialog-switch ${sanitize ? 'import-dialog-switch--on' : ''}`}
              onClick={() => setSanitize(!sanitize)}
              role="switch"
              aria-checked={sanitize}
            >
              <span className="import-dialog-switch-thumb" />
            </button>
          </label>
          <p className="import-dialog-hint">
            {sanitize
              ? 'Uses an AI agent to analyze and transform the skill for security before import.'
              : 'Direct import without security screening. Only use for trusted y-agent format skills.'}
          </p>
        </div>

        {/* Actions */}
        <div className="import-dialog-actions">
          <button className="import-dialog-btn import-dialog-btn--cancel" onClick={onClose}>
            Cancel
          </button>
          <button
            className="import-dialog-btn import-dialog-btn--import"
            onClick={handleImport}
            disabled={!path}
          >
            <Upload size={14} />
            Import
          </button>
        </div>
      </div>
    </div>
  );
}
