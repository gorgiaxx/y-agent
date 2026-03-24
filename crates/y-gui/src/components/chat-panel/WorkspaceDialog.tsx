import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, X } from 'lucide-react';
import './WorkspaceDialog.css';

interface WorkspaceDialogProps {
  onConfirm: (name: string, path: string) => void;
  onClose: () => void;
  /** If provided, opens in edit mode with the existing workspace data. */
  initialName?: string;
  initialPath?: string;
}

export function WorkspaceDialog({
  onConfirm,
  onClose,
  initialName = '',
  initialPath = '',
}: WorkspaceDialogProps) {
  const [name, setName] = useState(initialName);
  const [path, setPath] = useState(initialPath);
  const [picking, setPicking] = useState(false);

  const handlePickFolder = async () => {
    setPicking(true);
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && typeof selected === 'string') {
        setPath(selected);
        // Auto-fill name from folder base name if name is still empty.
        if (!name.trim()) {
          const parts = selected.replace(/\\/g, '/').split('/');
          setName(parts[parts.length - 1] || '');
        }
      }
    } finally {
      setPicking(false);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !path.trim()) return;
    onConfirm(name.trim(), path.trim());
  };

  return (
    <div className="ws-dialog-backdrop" onClick={onClose}>
      <div className="ws-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="ws-dialog-header">
          <span className="ws-dialog-title">{initialName ? 'Edit Workspace' : 'New Workspace'}</span>
          <button className="ws-dialog-close" onClick={onClose} aria-label="Close">
            <X size={14} />
          </button>
        </div>

        <form className="ws-dialog-body" onSubmit={handleSubmit}>
          <label className="ws-field-label">Name</label>
          <input
            className="ws-field-input"
            type="text"
            placeholder="My Project"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />

          <label className="ws-field-label" style={{ marginTop: '14px' }}>Folder</label>
          <div className="ws-path-row">
            <div className="ws-path-display" title={path}>
              {path || <span className="ws-path-placeholder">No folder selected</span>}
            </div>
            <button
              type="button"
              className="ws-pick-btn"
              onClick={handlePickFolder}
              disabled={picking}
            >
              <FolderOpen size={13} />
              {picking ? 'Opening...' : 'Choose'}
            </button>
          </div>

          <div className="ws-dialog-actions">
            <button type="button" className="ws-btn-cancel" onClick={onClose}>Cancel</button>
            <button
              type="submit"
              className="ws-btn-confirm"
              disabled={!name.trim() || !path.trim()}
            >
              {initialName ? 'Save' : 'Create'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
