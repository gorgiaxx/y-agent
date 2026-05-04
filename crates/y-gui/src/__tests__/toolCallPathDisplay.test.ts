import { describe, expect, it } from 'vitest';

import { formatToolResultPath } from '../components/chat-panel/chat-box/toolCallUtils';

describe('tool result path display', () => {
  it('keeps full paths for Glob and Grep match chips', () => {
    expect(formatToolResultPath('/workspace/app/package.json')).toBe('/workspace/app/package.json');
  });
});
