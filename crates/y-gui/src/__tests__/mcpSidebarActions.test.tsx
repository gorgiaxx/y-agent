import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { McpSidebarActions } from '../components/settings/McpTab';

describe('MCP settings sidebar actions', () => {
  it('keeps add and refresh controls in a fixed action row above long lists', () => {
    const html = renderToStaticMarkup(
      <McpSidebarActions
        statusLoading={false}
        onAdd={() => {}}
        onRefresh={() => {}}
      />,
    );

    expect(html).toContain('class="sub-list-actions"');
    expect(html).toContain('Add');
    expect(html).toContain('aria-label="Refresh connection status"');
    expect(html).not.toContain('sub-list-item-refresh');
  });
});
