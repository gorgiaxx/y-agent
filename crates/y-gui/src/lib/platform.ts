// Platform abstraction for non-invoke Tauri APIs.
//
// Wraps file dialogs, URL opening, window controls, and app version
// with implementations that work in both Tauri and browser environments.

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

export interface Platform {
  openFileDialog(options?: OpenDialogOptions): Promise<string[] | null>;
  openUrl(url: string): Promise<void>;
  revealInFileManager(path: string): Promise<void>;
  getAppVersion(): Promise<string>;
  isTauri(): boolean;
}

class TauriPlatform implements Platform {
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

  async openUrl(url: string): Promise<void> {
    window.open(url, '_blank', 'noopener');
  }

  async revealInFileManager(_path: string): Promise<void> {
    void _path;
    // not available in browser
  }

  async getAppVersion(): Promise<string> {
    try {
      const resp = await fetch(`${this.apiUrl}/health`);
      const data = await resp.json();
      return data.version ?? 'unknown';
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

function getDefaultApiUrl(): string {
  return (
    (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/+$/, '')
    ?? 'http://localhost:3000'
  );
}

export const platform: Platform = createPlatform(getDefaultApiUrl());
