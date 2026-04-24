import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  MAX_BROWSER_ATTACHMENT_BYTES,
  createPlatform,
  fileToAttachment,
} from '../lib/platform';

describe('platform capabilities', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('advertises browser capabilities without native file paths', () => {
    vi.stubGlobal('window', {});

    const platform = createPlatform('http://localhost:3000');

    expect(platform.isTauri()).toBe(false);
    expect(platform.capabilities).toMatchObject({
      nativeWindowControls: false,
      nativeFilePaths: false,
      browserFileUpload: true,
      revealFileManager: false,
      skillImportFromPath: true,
      knowledgeIngestFromPath: true,
      remoteAuth: true,
      sseEvents: true,
    });
  });

  it('advertises native capabilities when Tauri internals are present', () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });

    const platform = createPlatform('http://localhost:3000');

    expect(platform.isTauri()).toBe(true);
    expect(platform.capabilities).toMatchObject({
      nativeWindowControls: true,
      nativeFilePaths: true,
      browserFileUpload: false,
      revealFileManager: true,
      skillImportFromPath: true,
      knowledgeIngestFromPath: true,
      remoteAuth: false,
      sseEvents: false,
    });
  });

  it('converts browser Files into shared attachment payloads', async () => {
    const file = new File(['hello'], 'hello.png', { type: 'image/png' });

    const attachment = await fileToAttachment(file);

    expect(attachment.filename).toBe('hello.png');
    expect(attachment.mime_type).toBe('image/png');
    expect(attachment.size).toBe(5);
    expect(attachment.base64_data).toBe('aGVsbG8=');
    expect(attachment.id).toMatch(/^browser-/);
  });

  it('rejects oversized browser attachments before sending a chat request', async () => {
    const file = new File(
      [new Uint8Array(MAX_BROWSER_ATTACHMENT_BYTES + 1)],
      'too-large.png',
      { type: 'image/png' },
    );

    await expect(fileToAttachment(file)).rejects.toThrow('exceeds the 20 MB attachment limit');
  });
});
