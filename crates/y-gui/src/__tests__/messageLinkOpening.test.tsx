import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { makeMarkdownComponents } from '../components/chat-panel/chat-box/messageUtils';
import { isAbsoluteFilePath, isFileUrl, isLocalPath, resolveLocalPath } from '../components/chat-panel/chat-box/linkUtils';

const mockPlatform = vi.hoisted(() => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
  openPathInIde: vi.fn().mockResolvedValue(undefined),
  revealInFileManager: vi.fn().mockResolvedValue(undefined),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
  isTauri: () => true,
  capabilities: {
    openInIde: true,
    revealFileManager: true,
    nativeWindowControls: true,
    nativeFilePaths: true,
    browserFileUpload: false,
    skillImportFromPath: true,
    knowledgeIngestFromPath: true,
    remoteAuth: false,
    sseEvents: false,
  },
}));

vi.mock('../lib/platform', () => ({
  platform: mockPlatform,
}));

vi.mock('../lib', () => ({
  transport: { invoke: vi.fn() },
  platform: mockPlatform,
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}));

describe('link detection utilities', () => {
  it('detects Unix absolute paths', () => {
    expect(isAbsoluteFilePath('/Users/test/file.rs')).toBe(true);
    expect(isAbsoluteFilePath('/tmp/data.json')).toBe(true);
  });

  it('detects Windows absolute paths', () => {
    expect(isAbsoluteFilePath('C:\\Users\\test\\file.rs')).toBe(true);
    expect(isAbsoluteFilePath('D:/Projects/main.rs')).toBe(true);
  });

  it('rejects relative paths and URLs', () => {
    expect(isAbsoluteFilePath('src/main.rs')).toBe(false);
    expect(isAbsoluteFilePath('https://example.com')).toBe(false);
    expect(isAbsoluteFilePath('./file.txt')).toBe(false);
    expect(isAbsoluteFilePath(undefined)).toBe(false);
    expect(isAbsoluteFilePath(42)).toBe(false);
  });

  it('detects file:// URLs', () => {
    expect(isFileUrl('file:///Users/test/file.rs')).toBe(true);
    expect(isFileUrl('file:///tmp/data.json')).toBe(true);
    expect(isFileUrl('https://example.com')).toBe(false);
    expect(isFileUrl('/Users/test/file.rs')).toBe(false);
  });

  it('isLocalPath matches both bare paths and file:// URLs', () => {
    expect(isLocalPath('/Users/test/file.rs')).toBe(true);
    expect(isLocalPath('file:///Users/test/file.rs')).toBe(true);
    expect(isLocalPath('C:\\Users\\test\\file.rs')).toBe(true);
    expect(isLocalPath('https://example.com')).toBe(false);
  });

  it('resolveLocalPath strips file:// prefix', () => {
    expect(resolveLocalPath('file:///Users/test/file.rs')).toBe('/Users/test/file.rs');
    expect(resolveLocalPath('/Users/test/file.rs')).toBe('/Users/test/file.rs');
  });
});

describe('message markdown link rendering', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders web links with md-link-web class', () => {
    const components = makeMarkdownComponents({});
    type AnchorRenderer = (props: { href?: string; children?: React.ReactNode; node?: unknown }) => React.ReactElement;
    const Anchor = (components as Record<string, unknown>).a as AnchorRenderer;
    const html = renderToStaticMarkup(<Anchor href="https://example.com">docs</Anchor>);
    expect(html).toContain('md-link-web');
    expect(html).toContain('docs');
  });

  it('renders local file links as a tag with full path title', () => {
    const components = makeMarkdownComponents({});
    type AnchorRenderer = (props: { href?: string; children?: React.ReactNode; node?: unknown }) => React.ReactElement;
    const Anchor = (components as Record<string, unknown>).a as AnchorRenderer;
    const html = renderToStaticMarkup(
      <Anchor href="/Users/test/project/GNUmakefile">GNUmakefile</Anchor>,
    );
    expect(html).toContain('md-file-tag');
    expect(html).toContain('title="/Users/test/project/GNUmakefile"');
    expect(html).toContain('md-file-tag-actions');
  });

  it('renders file:// URLs as file tags with resolved path', () => {
    const components = makeMarkdownComponents({});
    type AnchorRenderer = (props: { href?: string; children?: React.ReactNode; node?: unknown }) => React.ReactElement;
    const Anchor = (components as Record<string, unknown>).a as AnchorRenderer;
    const html = renderToStaticMarkup(
      <Anchor href="file:///Users/gorgias/RE/igs/GNUmakefile">GNUmakefile</Anchor>,
    );
    expect(html).toContain('md-file-tag');
    expect(html).toContain('title="/Users/gorgias/RE/igs/GNUmakefile"');
    expect(html).toContain('md-file-tag-actions');
  });

  it('renders image file links with image icon', () => {
    const components = makeMarkdownComponents({});
    type AnchorRenderer = (props: { href?: string; children?: React.ReactNode; node?: unknown }) => React.ReactElement;
    const Anchor = (components as Record<string, unknown>).a as AnchorRenderer;
    const html = renderToStaticMarkup(
      <Anchor href="/Users/test/photo.jpg">photo.jpg</Anchor>,
    );
    expect(html).toContain('md-file-tag');
  });

  it('renders custom img with skeleton loading state', () => {
    const components = makeMarkdownComponents({});
    expect(components).toHaveProperty('img');
    type ImgRenderer = (props: { src?: string; alt?: string; node?: unknown }) => React.ReactElement;
    const Img = (components as Record<string, unknown>).img as ImgRenderer;
    const html = renderToStaticMarkup(
      <Img src="https://example.com/test.png" alt="test image" />,
    );
    expect(html).toContain('<img');
    expect(html).toContain('md-image');
    expect(html).toContain('md-image-skeleton');
    expect(html).toContain('alt="test image"');
  });

  it('renders local images through convertFileSrc for Tauri', () => {
    const components = makeMarkdownComponents({});
    type ImgRenderer = (props: { src?: string; alt?: string; node?: unknown }) => React.ReactElement;
    const Img = (components as Record<string, unknown>).img as ImgRenderer;
    const html = renderToStaticMarkup(
      <Img src="/Users/test/Downloads/screenshot.png" alt="screenshot" />,
    );
    expect(mockPlatform.convertFileSrc).toHaveBeenCalledWith('/Users/test/Downloads/screenshot.png');
    expect(html).toContain('md-image-skeleton');
    expect(html).toContain('md-image-loading');
  });

  it('renders file:// image URLs through convertFileSrc with resolved path', () => {
    mockPlatform.convertFileSrc.mockClear();
    const components = makeMarkdownComponents({});
    type ImgRenderer = (props: { src?: string; alt?: string; node?: unknown }) => React.ReactElement;
    const Img = (components as Record<string, unknown>).img as ImgRenderer;
    const html = renderToStaticMarkup(
      <Img src="file:///Users/gorgias/Downloads/photo.jpg" alt="photo" />,
    );
    expect(mockPlatform.convertFileSrc).toHaveBeenCalledWith('/Users/gorgias/Downloads/photo.jpg');
    expect(html).toContain('md-image-skeleton');
  });

  it('renders non-local images directly without convertFileSrc', () => {
    mockPlatform.convertFileSrc.mockClear();
    const components = makeMarkdownComponents({});
    type ImgRenderer = (props: { src?: string; alt?: string; node?: unknown }) => React.ReactElement;
    const Img = (components as Record<string, unknown>).img as ImgRenderer;
    renderToStaticMarkup(
      <Img src="https://cdn.example.com/logo.png" alt="logo" />,
    );
    expect(mockPlatform.convertFileSrc).not.toHaveBeenCalled();
  });
});
