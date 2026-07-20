import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { BackgroundWakeFields } from '../components/settings/BackgroundWakeTab';
import {
  DEFAULT_BACKGROUND_WAKE_FORM,
  jsonToBackgroundWake,
} from '../components/settings/settingsTypes';
import { BACKGROUND_WAKE_SCHEMA } from '../utils/settingsSchemas';
import { serializeToml } from '../utils/tomlUtils';

describe('background auto-wake settings', () => {
  it('round-trips the bounded wake policy fields', () => {
    const form = jsonToBackgroundWake({
      enabled: true,
      max_wakes_per_hour: 4,
      cooldown_secs: 90,
      allow_during_orchestration: true,
    });

    const toml = serializeToml(form as unknown as Record<string, unknown>, BACKGROUND_WAKE_SCHEMA);

    expect(toml).toContain('enabled = true');
    expect(toml).toContain('max_wakes_per_hour = 4');
    expect(toml).toContain('cooldown_secs = 90');
    expect(toml).toContain('allow_during_orchestration = true');
  });

  it('disables controls and explains unavailable builds', () => {
    const html = renderToStaticMarkup(
      <BackgroundWakeFields
        form={DEFAULT_BACKGROUND_WAKE_FORM}
        availability={{ available: false, restart_required: true }}
        onUpdate={() => {}}
      />,
    );

    expect(html).toContain('Background auto-wake is not compiled into this binary');
    expect(html).toContain('Enable Automatic Wake');
    expect(html).toContain('disabled=""');
    expect(html).toContain('Allow During Plan or Loop Execution');
  });
});
