import { Copy, ExternalLink, FileText, FolderOpen } from 'lucide-react';

import { logger, platform } from '../../../lib';
import type { ContextMenuItem } from './useContextMenu';

interface FileContextMenuOptions {
  openInIde?: boolean;
  openFile?: boolean;
  revealInFileManager?: boolean;
  copyPath?: boolean;
}

function fileUrl(path: string): string {
  return `file://${path}`;
}

function copyPathToClipboard(path: string): void {
  if (typeof navigator === 'undefined' || !navigator.clipboard) {
    return;
  }
  navigator.clipboard.writeText(path).catch((error) =>
    logger.error('[FileContextMenu] failed to copy path:', path, error),
  );
}

export function buildFileContextMenuItems(
  path: string,
  options: FileContextMenuOptions = {},
): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];

  if (options.openInIde) {
    items.push({
      label: 'Open in IDE',
      icon: <ExternalLink size={14} />,
      onClick: () => {
        platform.openPathInIde(path).catch((error) =>
          logger.error('[FileContextMenu] failed to open in IDE:', path, error),
        );
      },
    });
  }

  if (options.openFile) {
    items.push({
      label: 'Open File',
      icon: <FileText size={14} />,
      onClick: () => {
        platform.openUrl(fileUrl(path)).catch((error) =>
          logger.error('[FileContextMenu] failed to open file:', path, error),
        );
      },
    });
  }

  if (options.revealInFileManager) {
    items.push({
      label: 'Open Containing Folder',
      icon: <FolderOpen size={14} />,
      onClick: () => {
        platform.revealInFileManager(path).catch((error) =>
          logger.error('[FileContextMenu] failed to reveal:', path, error),
        );
      },
    });
  }

  if (options.copyPath) {
    items.push({
      label: 'Copy Path',
      icon: <Copy size={14} />,
      onClick: () => copyPathToClipboard(path),
    });
  }

  return items;
}
