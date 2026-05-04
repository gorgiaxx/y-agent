export interface HostDataset {
  host: 'tauri' | 'web';
  platform: 'macos' | 'other';
}

export function resolveHostDataset(
  isTauri: boolean,
  navigatorPlatform: string | undefined,
): HostDataset {
  return {
    host: isTauri ? 'tauri' : 'web',
    platform: isTauri && navigatorPlatform && /Mac/i.test(navigatorPlatform) ? 'macos' : 'other',
  };
}
