import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Button,
  Input,
} from '../ui';

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
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent width="400px" className="text-left items-stretch">
        <DialogTitle className="text-left">
          {initialName ? 'Edit Workspace' : 'New Workspace'}
        </DialogTitle>

        <form className="flex flex-col gap-3 mt-2" onSubmit={handleSubmit}>
          <div className="flex flex-col gap-1">
            <label className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
              Name
            </label>
            <Input
              type="text"
              placeholder="My Project"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </div>

          <div className="flex flex-col gap-1">
            <label className="text-10px font-500 text-[var(--text-muted)] uppercase tracking-[0.06em]">
              Folder
            </label>
            <div className="flex gap-2 items-center">
              <div
                className={[
                  'flex-1 min-w-0 px-2 py-1.5',
                  'text-12px',
                  'border border-solid border-[var(--border)]',
                  'rounded-[var(--radius-sm)]',
                  'bg-[var(--surface-secondary)]',
                  'overflow-hidden text-ellipsis whitespace-nowrap',
                  path ? 'text-[var(--text-primary)]' : 'text-[var(--text-muted)]',
                ].join(' ')}
                title={path}
              >
                {path || 'No folder selected'}
              </div>
              <Button
                type="button"
                variant="outline"
                onClick={handlePickFolder}
                disabled={picking}
              >
                <FolderOpen size={13} />
                {picking ? 'Opening...' : 'Choose'}
              </Button>
            </div>
          </div>

          <div className="flex gap-2 justify-end mt-1">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              disabled={!name.trim() || !path.trim()}
            >
              {initialName ? 'Save' : 'Create'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
