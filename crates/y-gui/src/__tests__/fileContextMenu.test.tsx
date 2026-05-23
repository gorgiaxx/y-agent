import { describe, expect, it, vi, beforeEach } from 'vitest';

const mockPlatform = vi.hoisted(() => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
  openPathInIde: vi.fn().mockResolvedValue(undefined),
  revealInFileManager: vi.fn().mockResolvedValue(undefined),
  capabilities: {
    openInIde: true,
    revealFileManager: true,
  },
}));

vi.mock('../lib', () => ({
  platform: {
    capabilities: mockPlatform.capabilities,
    openUrl: mockPlatform.openUrl,
    openPathInIde: mockPlatform.openPathInIde,
    revealInFileManager: mockPlatform.revealInFileManager,
  },
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}));

import { buildFileContextMenuItems } from '../components/chat-panel/chat-box/fileContextMenu';

describe('buildFileContextMenuItems', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('builds file and folder actions for read file tool results', () => {
    const items = buildFileContextMenuItems('/tmp/example.ts', {
      openInIde: false,
      openFile: true,
      revealInFileManager: true,
      copyPath: true,
    });

    expect(items.map((item) => item.label)).toEqual([
      'Open File',
      'Open Containing Folder',
      'Copy Path',
    ]);

    items[0].onClick();
    items[1].onClick();

    expect(mockPlatform.openUrl).toHaveBeenCalledWith('file:///tmp/example.ts');
    expect(mockPlatform.revealInFileManager).toHaveBeenCalledWith('/tmp/example.ts');
  });

  it('builds IDE, file, and folder actions for create and update file tool results', () => {
    const items = buildFileContextMenuItems('/tmp/example.ts', {
      openInIde: true,
      openFile: true,
      revealInFileManager: true,
      copyPath: true,
    });

    expect(items.map((item) => item.label)).toEqual([
      'Open in IDE',
      'Open File',
      'Open Containing Folder',
      'Copy Path',
    ]);

    items[0].onClick();
    expect(mockPlatform.openPathInIde).toHaveBeenCalledWith('/tmp/example.ts');
  });
});
