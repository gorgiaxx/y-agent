import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import {
  CapabilityPackInspectionPanel,
  InstalledCapabilityPackCard,
} from '../components/settings/CapabilityPacksTab';
import { SearchableOptionList } from '../components/common/SearchableSelect';
import { filterSearchableOptions } from '../components/common/searchableSelectUtils';
import { buildApprovalSessionOptions } from '../components/settings/capabilityPackViewModel';
import type {
  CapabilityPackInspection,
  InstalledCapabilityPackSummary,
  SessionInfo,
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
  it('supports searchable approval sessions without a long select menu', () => {
    const sessions: SessionInfo[] = Array.from({ length: 40 }, (_, index) => ({
      id: `session-${index}`,
      agent_id: index === 27 ? 'release-agent' : 'default',
      title: index === 27 ? 'Needle deployment review' : `Session ${index}`,
      manual_title: null,
      created_at: '2026-07-20T00:00:00Z',
      updated_at: '2026-07-20T00:00:00Z',
      message_count: index,
    }));
    const options = buildApprovalSessionOptions(sessions);

    expect(filterSearchableOptions(options, 'needle')).toHaveLength(1);
    expect(filterSearchableOptions(options, 'release-agent')[0]?.value).toBe('session-27');
    expect(filterSearchableOptions(options, 'session-27')[0]?.label).toBe('Needle deployment review');

    const html = renderToStaticMarkup(
      <SearchableOptionList
        options={options}
        query="needle"
        searchPlaceholder="Search approval sessions..."
        emptyMessage="No matching sessions"
        selectedValue="session-27"
        onQueryChange={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(html).toContain('Search approval sessions...');
    expect(html).toContain('Needle deployment review');
    expect(html).not.toContain('Session 12');
  });

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
