import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { SessionAdvancedFields } from '../components/settings/SessionTab';
import { jsonToSession } from '../components/settings/settingsTypes';
import { SESSION_SCHEMA } from '../utils/settingsSchemas';
import { mergeIntoRawToml } from '../utils/tomlUtils';

describe('advanced session settings', () => {
  it('deserializes compaction prefire and intra-turn pruning fields', () => {
    const form = jsonToSession({
      compaction_prefire_threshold_pct: 72,
      pruning: {
        intra_turn: {
          enabled: false,
          min_iteration: 4,
          token_threshold: 1500,
        },
      },
    });

    expect(form.compaction_prefire_threshold_pct).toBe(72);
    expect(form.pruning_intra_turn_enabled).toBe(false);
    expect(form.pruning_intra_turn_min_iteration).toBe(4);
    expect(form.pruning_intra_turn_token_threshold).toBe(1500);
  });

  it('merges the missing fields into their canonical TOML sections', () => {
    const form = jsonToSession({});
    const toml = mergeIntoRawToml('max_depth = 16\n\n[pruning]\nenabled = true\n', {
      ...form,
      compaction_prefire_threshold_pct: 74,
      pruning_intra_turn_min_iteration: 5,
    }, SESSION_SCHEMA);

    expect(toml).toContain('compaction_prefire_threshold_pct = 74');
    expect(toml).toContain('[pruning.intra_turn]');
    expect(toml).toContain('min_iteration = 5');
  });

  it('renders feature availability and intra-turn controls in the form', () => {
    const html = renderToStaticMarkup(
      <SessionAdvancedFields
        form={jsonToSession({})}
        prefireAvailability={{ available: false, restart_required: false }}
        onUpdate={() => {}}
      />,
    );

    expect(html).toContain('Compaction Prefire Threshold');
    expect(html).toContain('not compiled into this binary');
    expect(html).toContain('Intra-turn Pruning');
    expect(html).toContain('Minimum Iteration');
  });
});
