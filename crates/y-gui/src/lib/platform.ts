// Platform abstraction for non-invoke Tauri APIs.
//
// Wraps file dialogs, URL opening, window controls, and app version
// with implementations that work in both Tauri and browser environments.

import type { Attachment } from '../types';

export interface FileFilter {
  name: string;
  extensions: string[];
}

export interface OpenDialogOptions {
  multiple?: boolean;
  directory?: boolean;
  filters?: FileFilter[];
  title?: string;
}

export interface PlatformCapabilities {
  nativeWindowControls: boolean;
  nativeFilePaths: boolean;
  browserFileUpload: boolean;
  revealFileManager: boolean;
  skillImportFromPath: boolean;
  knowledgeIngestFromPath: boolean;
  remoteAuth: boolean;
  sseEvents: boolean;
}

export const MAX_BROWSER_ATTACHMENT_BYTES = 20 * 1024 * 1024;

export interface Platform {
  readonly capabilities: PlatformCapabilities;
  openFileDialog(options?: OpenDialogOptions): Promise<string[] | null>;
  openImageAttachments(options?: OpenDialogOptions): Promise<Attachment[] | null>;
  openUrl(url: string): Promise<void>;
  revealInFileManager(path: string): Promise<void>;
  getAppVersion(): Promise<string>;
  isTauri(): boolean;
}

class TauriPlatform implements Platform {
  readonly capabilities: PlatformCapabilities = {
    nativeWindowControls: true,
    nativeFilePaths: true,
    browserFileUpload: false,
    revealFileManager: true,
    skillImportFromPath: true,
    knowledgeIngestFromPath: true,
    remoteAuth: false,
    sseEvents: false,
  };

  async openFileDialog(options?: OpenDialogOptions): Promise<string[] | null> {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({
      multiple: options?.multiple,
      directory: options?.directory,
      filters: options?.filters,
      title: options?.title,
    });
    if (result === null) return null;
    if (typeof result === 'string') return [result];
    if (Array.isArray(result)) {
      return (result as Array<string | { path: string }>).map((r) => typeof r === 'string' ? r : r.path);
    }
    if (typeof result === 'object' && result !== null && 'path' in result) {
      return [(result as { path: string }).path];
    }
    return [String(result)];
  }

  async openImageAttachments(): Promise<Attachment[] | null> {
    return null;
  }

  async openUrl(url: string): Promise<void> {
    const { openUrl } = await import('@tauri-apps/plugin-opener');
    await openUrl(url);
  }

  async revealInFileManager(path: string): Promise<void> {
    const { revealItemInDir } = await import('@tauri-apps/plugin-opener');
    await revealItemInDir(path);
  }

  async getAppVersion(): Promise<string> {
    const { getVersion } = await import('@tauri-apps/api/app');
    return getVersion();
  }

  isTauri(): boolean {
    return true;
  }
}

class WebPlatform implements Platform {
  readonly capabilities: PlatformCapabilities = {
    nativeWindowControls: false,
    nativeFilePaths: false,
    browserFileUpload: true,
    revealFileManager: false,
    skillImportFromPath: true,
    knowledgeIngestFromPath: true,
    remoteAuth: true,
    sseEvents: true,
  };

  private apiUrl: string;

  constructor(apiUrl: string) {
    this.apiUrl = apiUrl;
  }

  async openFileDialog(options?: OpenDialogOptions): Promise<string[] | null> {
    return new Promise((resolve) => {
      const input = document.createElement('input');
      input.type = 'file';
      if (options?.multiple) input.multiple = true;
      if (options?.directory) {
        input.setAttribute('webkitdirectory', '');
      }
      if (options?.filters?.length) {
        const exts = options.filters.flatMap((f) => f.extensions.map((e) => `.${e}`));
        input.accept = exts.join(',');
      }
      input.onchange = () => {
        if (!input.files?.length) {
          resolve(null);
          return;
        }
        const paths = Array.from(input.files).map((f) => f.name);
        resolve(paths);
      };
      input.oncancel = () => resolve(null);
      input.click();
    });
  }

  async openImageAttachments(options?: OpenDialogOptions): Promise<Attachment[] | null> {
    return new Promise((resolve, reject) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.multiple = options?.multiple ?? true;
      const filters = options?.filters ?? [
        { name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] },
      ];
      input.accept = filters
        .flatMap((filter) => filter.extensions.map((extension) => `.${extension}`))
        .join(',');
      input.onchange = () => {
        if (!input.files?.length) {
          resolve(null);
          return;
        }
        Promise.all(Array.from(input.files).map(fileToAttachment))
          .then(resolve)
          .catch(reject);
      };
      input.oncancel = () => resolve(null);
      input.click();
    });
  }

  async openUrl(url: string): Promise<void> {
    window.open(url, '_blank', 'noopener');
  }

  async revealInFileManager(_path: string): Promise<void> {
    void _path;
    throw new Error('Reveal in file manager is not supported in the browser');
  }

  async getAppVersion(): Promise<string> {
    try {
      const resp = await fetch(`${this.apiUrl}/health`);
      const data = await resp.json();
      return data.app_version ?? data.version ?? 'unknown';
    } catch {
      return 'unknown';
    }
  }

  isTauri(): boolean {
    return false;
  }
}

export function createPlatform(apiUrl: string): Platform {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    return new TauriPlatform();
  }
  return new WebPlatform(apiUrl);
}

function arrayBufferToBase64(buffer: ArrayBuffer) {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

function createAttachmentId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return `browser-${crypto.randomUUID()}`;
  }
  return `browser-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export async function fileToAttachment(file: File): Promise<Attachment> {
  if (file.size > MAX_BROWSER_ATTACHMENT_BYTES) {
    throw new Error(`${file.name} exceeds the 20 MB attachment limit`);
  }
  const buffer = await file.arrayBuffer();
  return {
    id: createAttachmentId(),
    filename: file.name,
    mime_type: file.type || 'application/octet-stream',
    base64_data: arrayBufferToBase64(buffer),
    size: file.size,
  };
}

function getDefaultApiUrl(): string {
  return (
    (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/+$/, '')
    ?? 'http://localhost:3000'
  );
}

export const platform: Platform = createPlatform(getDefaultApiUrl());
