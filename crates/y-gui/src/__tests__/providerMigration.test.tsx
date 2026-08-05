// @vitest-environment happy-dom
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from 'vitest';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';

import { ProviderMigrationPanel } from '../components/common/ProviderMigrationPanel';
import type {
  MigrationReport,
  MigrationSourceInfo,
} from '../types';

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const detectFixture: MigrationSourceInfo[] = [
  {
    id: 'omp',
    label: 'omp',
    icon_id: null,
    detected: true,
    migrated: false,
    supported: true,
    unsupported_reason: null,
    providers: [
      {
        id: 'omp-gpt4o',
        label: 'GPT-4o (omp)',
        provider_type: 'openai-compat',
        model: 'gpt-4o',
        base_url: 'https://api.openai.com/v1',
        has_api_key: true,
        api_key_preview: 'sk-4054...cb7',
        context_window: 128000,
        source_provider_name: 'gpt-4o',
      },
      {
        id: 'omp-claude',
        label: 'Claude 3.5 (omp)',
        provider_type: 'anthropic',
        model: 'claude-3-5-sonnet',
        base_url: null,
        has_api_key: false,
        api_key_preview: '',
        context_window: 200000,
        source_provider_name: 'claude-3-5-sonnet',
      },
    ],
  },
  {
    id: 'claude',
    label: 'Claude',
    icon_id: 'Anthropic',
    detected: true,
    migrated: true,
    supported: true,
    unsupported_reason: null,
    providers: [],
  },
  {
    id: 'codex',
    label: 'Codex',
    icon_id: 'OpenAI',
    detected: true,
    migrated: false,
    supported: false,
    unsupported_reason: 'Codex 使用 OAuth 登录，无法迁移 API key',
    providers: [],
  },
  {
    id: 'kimi',
    label: 'Kimi',
    icon_id: 'Moonshot',
    detected: false,
    migrated: false,
    supported: true,
    unsupported_reason: null,
    providers: [],
  },
  // omo intentionally absent: the panel must still render its fallback tile.
];

const runReport: MigrationReport = {
  source_id: 'omp',
  imported: ['omp-gpt4o'],
  skipped: ['omp-claude'],
  errors: [],
};

// ---------------------------------------------------------------------------
// Mock transport.invoke to route the two migration commands to fixtures.
// ---------------------------------------------------------------------------

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('../lib', () => ({
  transport: { invoke: invokeMock },
}));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'provider_migration_detect') return detectFixture;
    if (cmd === 'provider_migration_run') return runReport;
    return null;
  });
});

afterEach(() => {
  cleanup();
});

async function waitForDetect() {
  await waitFor(() => {
    expect(screen.getByTestId('migration-tile-omp').getAttribute('data-state')).toBe('actionable');
  });
}

describe('ProviderMigrationPanel', () => {
  it('renders all five source logos with their detection states', async () => {
    render(<ProviderMigrationPanel />);

    await waitForDetect();

    const ids = ['omp', 'kimi', 'claude', 'codex', 'omo'] as const;
    for (const id of ids) {
      expect(screen.getByTestId(`migration-tile-${id}`)).toBeTruthy();
    }

    // omp: detected + supported + not migrated -> actionable
    expect(screen.getByTestId('migration-tile-omp').getAttribute('data-state')).toBe('actionable');

    // claude: migrated -> locked with a check badge
    const claudeTile = screen.getByTestId('migration-tile-claude');
    expect(claudeTile.getAttribute('data-state')).toBe('migrated');
    expect(claudeTile.querySelector('[aria-label="已迁移"]')).not.toBeNull();

    // codex: detected but unsupported -> dimmed, not clickable
    const codexTile = screen.getByTestId('migration-tile-codex');
    expect(codexTile.getAttribute('data-state')).toBe('unsupported');
    expect(codexTile.className).toContain('provider-migration-tile--dimmed');
    expect(codexTile.hasAttribute('disabled')).toBe(true);

    // kimi: not detected -> dimmed, not clickable
    const kimiTile = screen.getByTestId('migration-tile-kimi');
    expect(kimiTile.getAttribute('data-state')).toBe('not-detected');
    expect(kimiTile.className).toContain('provider-migration-tile--dimmed');
    expect(kimiTile.hasAttribute('disabled')).toBe(true);

    // omo: absent from detection -> falls back to a not-detected badge tile
    const omoTile = screen.getByTestId('migration-tile-omo');
    expect(omoTile.getAttribute('data-state')).toBe('not-detected');
  });

  it('does not open a dialog for locked, unsupported, or not-detected tiles', async () => {
    render(<ProviderMigrationPanel />);

    await waitForDetect();

    expect(screen.queryByRole('dialog')).toBeNull();

    // migrated, unsupported, and not-detected tiles are disabled and have no
    // click handler, so clicking them must never open the selection dialog.
    fireEvent.click(screen.getByTestId('migration-tile-claude'));
    expect(screen.queryByRole('dialog')).toBeNull();

    fireEvent.click(screen.getByTestId('migration-tile-codex'));
    expect(screen.queryByRole('dialog')).toBeNull();

    fireEvent.click(screen.getByTestId('migration-tile-kimi'));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens the omp selection dialog and migrates only checked candidates', async () => {
    const onMigrated = vi.fn();
    const setToast = vi.fn();

    render(<ProviderMigrationPanel onMigrated={onMigrated} setToast={setToast} />);

    await waitForDetect();

    // The actionable omp tile opens the selection dialog.
    fireEvent.click(screen.getByTestId('migration-tile-omp'));
    expect(await screen.findByRole('dialog')).toBeTruthy();

    // Both candidates are listed and checked by default.
    expect(screen.getByText('GPT-4o (omp)')).toBeTruthy();
    expect(screen.getByText('Claude 3.5 (omp)')).toBeTruthy();
    expect(screen.getByText('key: sk-4054...cb7')).toBeTruthy();
    expect(screen.getByText('无 API key')).toBeTruthy();

    const checkboxes = screen.getAllByRole('checkbox');
    expect(checkboxes).toHaveLength(2);
    expect(checkboxes[0].getAttribute('data-state')).toBe('checked');
    expect(checkboxes[1].getAttribute('data-state')).toBe('checked');

    // Uncheck the second candidate; only the first should be migrated.
    fireEvent.click(checkboxes[1]);
    await waitFor(() => {
      expect(checkboxes[1].getAttribute('data-state')).toBe('unchecked');
    });

    invokeMock.mockClear();

    fireEvent.click(screen.getByTestId('migration-confirm'));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('provider_migration_run', {
        sourceId: 'omp',
        selectedIds: ['omp-gpt4o'],
      });
    });

    // Host notification + success toast fire after a successful migration.
    await waitFor(() => {
      expect(onMigrated).toHaveBeenCalledWith(runReport);
    });
    expect(setToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success' }),
    );

    // The dialog closes once the migration succeeds.
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull();
    });
  });
});
