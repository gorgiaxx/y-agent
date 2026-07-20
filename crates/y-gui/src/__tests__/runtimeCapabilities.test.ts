import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock('../lib', () => ({
  transport: { invoke: invokeMock },
}));

import { loadRuntimeCapabilities } from '../hooks/useRuntimeCapabilities';
import { FeatureAvailabilityNotice } from '../components/settings/FeatureAvailabilityNotice';

describe('runtime capability negotiation', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('loads the service-owned optional subsystem contract', async () => {
    const expected = {
      background_auto_wake: { available: true, restart_required: true },
      lsp: { available: false, restart_required: true },
      capability_packs: { available: true, restart_required: false },
      hook_handlers: { available: false, restart_required: false },
      llm_hooks: { available: false, restart_required: false },
      compaction_prefire: { available: true, restart_required: false },
    };
    invokeMock.mockResolvedValue(expected);

    await expect(loadRuntimeCapabilities()).resolves.toEqual(expected);
    expect(invokeMock).toHaveBeenCalledWith('runtime_capabilities');
  });

  it('reports capability negotiation failures instead of remaining in a checking state', () => {
    const html = renderToStaticMarkup(createElement(FeatureAvailabilityNotice, {
      featureName: 'Capability Packs',
      availability: null,
      error: 'service unavailable',
      plural: true,
    }));

    expect(html).toContain('could not be confirmed');
    expect(html).toContain('service unavailable');
    expect(html).toContain('controls remain read-only');
    expect(html).not.toContain('Checking whether');
  });
});
