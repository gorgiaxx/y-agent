import { describe, expect, it } from 'vitest';

import { TOOL_RENDERERS } from '../components/chat-panel/chat-box/tool-renderers';

describe('tool renderer registry', () => {
  it('uses the current Plan renderer pipeline only', () => {
    expect(TOOL_RENDERERS).toHaveProperty('Plan');
  });
});
