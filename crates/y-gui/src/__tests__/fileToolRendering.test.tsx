import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { vi } from 'vitest';

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

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';

describe('File tool rendering', () => {
  it('renders file write calls with the file tag layout', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'file-write-1',
          name: 'FileWrite',
          arguments: JSON.stringify({
            path: '/tmp/example.ts',
            content: 'export const value = 1;\n',
          }),
        }}
        status="success"
        result={JSON.stringify({
          ok: true,
          path: '/tmp/example.ts',
        })}
      />,
    );

    expect(html).toContain('tool-call-file-wrapper');
    expect(html).toContain('tool-call-tag');
    expect(html).toContain('Create');
    expect(html).toContain('example.ts');
    expect(html).not.toContain('tool-call-card');
  });

  it('renders an IDE open action before the expand control for writable file calls', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'file-edit-1',
          name: 'FileEdit',
          arguments: JSON.stringify({
            file_path: '/tmp/example.ts',
            old_string: 'const value = 1;',
            new_string: 'const value = 2;',
          }),
        }}
        status="success"
        result={JSON.stringify({
          ok: true,
          file_path: '/tmp/example.ts',
          action: 'edited',
        })}
      />,
    );

    expect(html).toContain('tool-call-file-open');
    expect(html).toContain('aria-label="Open example.ts in IDE"');
    expect(html.indexOf('tool-call-file-open')).toBeLessThan(html.indexOf('tool-call-chevron'));
  });

  it('marks FileRead tool tags as context menu targets', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'file-read-1',
          name: 'FileRead',
          arguments: JSON.stringify({ path: '/tmp/example.ts' }),
        }}
        status="success"
        result={JSON.stringify({ content: 'export const value = 1;\n' })}
      />,
    );

    expect(html).toContain('data-file-context-menu="true"');
  });
});
