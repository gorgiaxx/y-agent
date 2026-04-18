import { useState } from 'react';
import { Upload, Shield, FileText } from 'lucide-react';
import { platform } from '../../lib';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Button,
  Input,
  Switch,
} from '../ui';

interface SkillImportDialogProps {
  onImport: (path: string, sanitize: boolean) => void;
  onClose: () => void;
}

export function SkillImportDialog({ onImport, onClose }: SkillImportDialogProps) {
  const [path, setPath] = useState('');
  const [sanitize, setSanitize] = useState(true);

  const handleBrowse = async () => {
    try {
      const selected = await platform.openFileDialog({});
      if (selected && selected.length > 0) {
        setPath(selected[0]);
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
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent size="md" className="text-left items-stretch">
        <DialogTitle className="text-left">Import Skill</DialogTitle>

        {/* Path selection */}
        <div className="flex flex-col gap-1 mt-2">
          <label className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
            Source Path
          </label>
          <div className="flex gap-2">
            <Input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="Select Skill main file..."
              readOnly
              variant="mono"
              className="flex-1"
            />
            <Button variant="outline" onClick={handleBrowse}>
              <FileText size={14} />
              Browse
            </Button>
          </div>
          <p className="text-10px text-[var(--text-muted)] mt-0.5">
            Please select the Skill main file
          </p>
        </div>

        {/* Sanitize toggle */}
        <div className="flex flex-col gap-1 mt-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-1.5">
              <Shield size={14} className="text-[var(--accent)]" />
              <span className="text-12px font-500 text-[var(--text-primary)]">
                Security Check (Agent-assisted)
              </span>
            </div>
            <Switch
              checked={sanitize}
              onCheckedChange={setSanitize}
            />
          </div>
          <p className="text-10px text-[var(--text-muted)]">
            {sanitize
              ? 'Uses an AI agent to analyze and transform the skill for security before import.'
              : 'Direct import without security screening. Only use for trusted y-agent format skills.'}
          </p>
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-3 justify-end">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleImport}
            disabled={!path}
          >
            <Upload size={14} />
            Import
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
