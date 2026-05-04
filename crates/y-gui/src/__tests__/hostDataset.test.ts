import { describe, expect, it } from 'vitest';

import { resolveHostDataset } from '../lib/hostDataset';

describe('resolveHostDataset', () => {
  it('keeps browser macOS out of Tauri-only vibrancy styling', () => {
    expect(resolveHostDataset(false, 'MacIntel')).toEqual({
      host: 'web',
      platform: 'other',
    });
  });

  it('marks Tauri macOS for native vibrancy styling', () => {
    expect(resolveHostDataset(true, 'MacIntel')).toEqual({
      host: 'tauri',
      platform: 'macos',
    });
  });
});
