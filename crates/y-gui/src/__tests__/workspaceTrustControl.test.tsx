import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { WorkspaceTrustSection } from '../components/chat-panel/WorkspaceTrustControl';
import type { WorkspaceTrustDecision } from '../types';

const decision = (status: WorkspaceTrustDecision['status']): WorkspaceTrustDecision => ({
  canonical_path: '/repo/project',
  status,
  updated_at: status === 'unknown' ? null : '2026-07-18T00:00:00Z',
});

describe('WorkspaceTrustSection', () => {
  it('explains that workspace trust does not bypass permission or HITL checks', () => {
    const html = renderToStaticMarkup(
      <WorkspaceTrustSection
        decision={decision('unknown')}
        loading={false}
        error={null}
        busy={false}
        onSetTrust={() => {}}
      />,
    );

    expect(html).toContain('Unknown');
    expect(html).toContain('Trust project config');
    expect(html).toContain('Block project config');
    expect(html).toContain('does not bypass tool permissions or HITL');
  });

  it('renders the canonical identity and the inverse action for trusted workspaces', () => {
    const html = renderToStaticMarkup(
      <WorkspaceTrustSection
        decision={decision('trusted')}
        loading={false}
        error={null}
        busy={false}
        onSetTrust={() => {}}
      />,
    );

    expect(html).toContain('Trusted');
    expect(html).toContain('/repo/project');
    expect(html).toContain('Block project config');
    expect(html).not.toContain('Trust project config');
  });

  it('renders a recovery action when project configuration is explicitly blocked', () => {
    const html = renderToStaticMarkup(
      <WorkspaceTrustSection
        decision={decision('untrusted')}
        loading={false}
        error={null}
        busy={false}
        onSetTrust={() => {}}
      />,
    );

    expect(html).toContain('Blocked');
    expect(html).toContain('Trust project config');
    expect(html).not.toContain('Block project config');
  });
});
