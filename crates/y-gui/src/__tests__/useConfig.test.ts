import { describe, expect, it } from 'vitest';

import { defaultGuiConfig, normalizeGuiConfig } from '../hooks/useConfig';

describe('normalizeGuiConfig', () => {
  it('uses the default GUI config when web localStorage has no saved value', () => {
    expect(normalizeGuiConfig(null)).toEqual(defaultGuiConfig);
  });

  it('preserves saved GUI preferences while filling missing fields', () => {
    expect(normalizeGuiConfig({
      ...defaultGuiConfig,
      theme: 'light',
      setup_completed: true,
    })).toEqual({
      ...defaultGuiConfig,
      theme: 'light',
      setup_completed: true,
    });
  });

  it('defaults file opening to automatic IDE detection', () => {
    expect(defaultGuiConfig.default_file_ide).toBe('auto');
    expect(normalizeGuiConfig({
      ...defaultGuiConfig,
      default_file_ide: 'cursor',
    }).default_file_ide).toBe('cursor');
  });
});
