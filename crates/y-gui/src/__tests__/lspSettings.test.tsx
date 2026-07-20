import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { LspSettingsFields } from '../components/settings/LspTab';
import { jsonToLsp } from '../components/settings/settingsTypes';
import { LSP_SCHEMA } from '../utils/settingsSchemas';
import { serializeToml } from '../utils/tomlUtils';

describe('LSP settings', () => {
  it('round-trips server declarations including initialization options', () => {
    const form = jsonToLsp({
      enabled: true,
      request_timeout_ms: 9000,
      max_message_bytes: 1024,
      max_restarts: 2,
      restart_base_delay_ms: 100,
      servers: [
        {
          id: 'rust',
          command: 'rust-analyzer',
          args: [],
          language_id: 'rust',
          extensions: ['rs'],
          root_markers: ['Cargo.toml'],
          initialization_options: { check: { command: 'clippy' } },
        },
      ],
    });

    const toml = serializeToml(form as unknown as Record<string, unknown>, LSP_SCHEMA);

    expect(toml).toContain('[[servers]]');
    expect(toml).toContain('command = "rust-analyzer"');
    expect(toml).toContain('extensions = ["rs"]');
    expect(toml).toContain('initialization_options = { check = { command = "clippy" } }');
  });

  it('renders server matching controls and blocks unavailable builds', () => {
    const form = jsonToLsp({});
    const html = renderToStaticMarkup(
      <LspSettingsFields
        form={form}
        availability={{ available: false, restart_required: true }}
        onUpdate={() => {}}
      />,
    );

    expect(html).toContain('Language Server Protocol is not compiled into this binary');
    expect(html).toContain('rust-analyzer');
    expect(html).toContain('File Extensions');
    expect(html).toContain('Root Markers');
    expect(html).toContain('disabled=""');
  });
});
