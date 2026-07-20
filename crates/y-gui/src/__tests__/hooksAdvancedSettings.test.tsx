import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { HooksAdvancedFields } from '../components/settings/HooksTab';
import { jsonToHooks } from '../components/settings/settingsTypes';
import { HOOKS_SCHEMA } from '../utils/settingsSchemas';
import { serializeToml } from '../utils/tomlUtils';

describe('advanced hook settings', () => {
  it('round-trips handler activation, allowed directories, and verbosity', () => {
    const form = jsonToHooks({
      handlers_enabled: false,
      allowed_hook_dirs: ['/opt/y-agent/hooks'],
      verbosity: 'minimal',
    });

    const toml = serializeToml(form as unknown as Record<string, unknown>, HOOKS_SCHEMA);

    expect(toml).toContain('handlers_enabled = false');
    expect(toml).toContain('allowed_hook_dirs = ["/opt/y-agent/hooks"]');
    expect(toml).toContain('verbosity = "minimal"');
  });

  it('explains feature gates while keeping middleware settings separate', () => {
    const html = renderToStaticMarkup(
      <HooksAdvancedFields
        form={jsonToHooks({})}
        handlerAvailability={{ available: false, restart_required: false }}
        llmHookAvailability={{ available: false, restart_required: false }}
        onUpdate={() => {}}
      />,
    );

    expect(html).toContain('Hook handlers are not compiled into this binary');
    expect(html).toContain('Enable External Handlers');
    expect(html).toContain('Allowed Hook Directories');
    expect(html).toContain('Prompt and agent handlers require LLM hooks');
    expect(html).toContain('Handler groups remain editable in RAW TOML mode');
  });
});
