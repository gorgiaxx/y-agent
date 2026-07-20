import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import {
  CapabilityPackInspectionPanel,
  InstalledCapabilityPackCard,
} from '../components/settings/CapabilityPacksTab';
import type {
  CapabilityPackInspection,
  InstalledCapabilityPackSummary,
} from '../types';

const inspection: CapabilityPackInspection = {
  validation: {
    valid: true,
    pack: {
      schema_version: 1,
      id: 'rust-team',
      version: '1.0.0',
      description: 'Rust capabilities',
      provenance: {
        source_kind: 'local_directory',
        pack_root: '/packs/rust-team',
        manifest_path: '/packs/rust-team/capability-pack.toml',
        manifest_sha256: 'a'.repeat(64),
      },
      resources: [],
    },
    issues: [],
  },
  preview: {
    pack_id: 'rust-team',
    pack_version: '1.0.0',
    can_apply: true,
    changes: [
      {
        resource_kind: 'mcp',
        resource_id: 'rust-tools',
        change: 'add',
        requires_activation: true,
        current_sha256: null,
        desired_sha256: 'b'.repeat(64),
      },
    ],
  },
};

const installed: InstalledCapabilityPackSummary = {
  pack_id: 'rust-team',
  current_version: '1.0.0',
  current_transaction_id: 'transaction-1',
  installed_versions: ['1.0.0'],
  resources: ['mcp:rust-tools'],
  executable_resources: ['mcp:rust-tools'],
  activation_grants: [
    {
      pack_id: 'rust-team',
      pack_version: '1.0.0',
      transaction_id: 'transaction-1',
      canonical_workspace: '/repo/project',
      approved_at: '2026-07-18T00:00:00Z',
    },
  ],
  live_resources: [],
};

describe('Capability Pack management', () => {
  it('keeps declarative installation separate from executable activation', () => {
    const html = renderToStaticMarkup(
      <CapabilityPackInspectionPanel
        inspection={inspection}
        allowReplacements={false}
        installing={false}
        onAllowReplacementsChange={() => {}}
        onInstall={() => {}}
      />,
    );

    expect(html).toContain('rust-team');
    expect(html).toContain('Requires separate activation');
    expect(html).toContain('Install declarative resources');
    expect(html).not.toContain('Install and activate');
  });

  it('does not report an approval grant as a live owner', () => {
    const html = renderToStaticMarkup(
      <InstalledCapabilityPackCard
        pack={installed}
        selectedWorkspacePath="/repo/project"
        busy={false}
        onActivate={() => {}}
        onRevoke={() => {}}
        onRollback={() => {}}
        onRemove={() => {}}
      />,
    );

    expect(html).toContain('Activation approved, not live');
    expect(html).toContain('Revoke approval');
    expect(html).not.toContain('Live: mcp:rust-tools');
  });

  it('reports only owner-confirmed resources as live', () => {
    const html = renderToStaticMarkup(
      <InstalledCapabilityPackCard
        pack={{ ...installed, live_resources: ['mcp:rust-tools'] }}
        selectedWorkspacePath="/repo/project"
        busy={false}
        onActivate={() => {}}
        onRevoke={() => {}}
        onRollback={() => {}}
        onRemove={() => {}}
      />,
    );

    expect(html).toContain('Live: mcp:rust-tools');
  });
});
