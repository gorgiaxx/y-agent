import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('ChatView plan mode wiring', () => {
  it('controls plan mode at the view level instead of forcing fast mode', () => {
    const source = readFileSync(new URL('../views/ChatView.tsx', import.meta.url), 'utf8');

    expect(source).not.toContain("planMode: 'fast' as PlanMode");
    expect(source).toContain('const [planMode, setPlanMode]');
    expect(source).toContain('onPlanModeChange: handlePlanModeChange');
    expect(source).toContain('planMode,');
  });
});
