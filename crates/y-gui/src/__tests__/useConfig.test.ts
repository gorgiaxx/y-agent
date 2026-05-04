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
});
