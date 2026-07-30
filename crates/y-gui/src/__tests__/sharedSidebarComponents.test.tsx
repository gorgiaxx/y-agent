import { createRef } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

import { SidebarOperationStatus } from '../components/common/SidebarOperationStatus';
import { SidebarSearchHeader } from '../components/common/SidebarSearchHeader';

describe('shared sidebar components', () => {
  it('renders the shared searchable header contract', () => {
    const html = renderToStaticMarkup(
      <SidebarSearchHeader
        label="Collections"
        count={3}
        searchTitle="Search collections"
        searchPlaceholder="Search collections..."
        searchOpen
        searchQuery="docs"
        searchInputRef={createRef<HTMLInputElement>()}
        onSearchQueryChange={() => {}}
        onSearchToggle={() => {}}
        actions={<button type="button">Add</button>}
      />,
    );

    expect(html).toContain('Collections');
    expect(html).toContain('>3<');
    expect(html).toContain('value="docs"');
    expect(html).toContain('placeholder="Search collections..."');
    expect(html).toContain('aria-label="Search collections"');
    expect(html).toContain('>Add<');
  });

  it('renders operation-specific progress and cancellation', () => {
    const html = renderToStaticMarkup(
      <SidebarOperationStatus
        status="running"
        runningMessage="Importing 2/4..."
        successMessage="Import complete"
        onDismiss={() => {}}
        onCancel={vi.fn()}
      />,
    );

    expect(html).toContain('import-status--importing');
    expect(html).toContain('Importing 2/4...');
    expect(html).toContain('aria-label="Cancel"');
    expect(html).not.toContain('aria-label="Copy error"');
  });

  it('centralizes expandable error actions', () => {
    const html = renderToStaticMarkup(
      <SidebarOperationStatus
        status="error"
        runningMessage="Importing..."
        successMessage="Import complete"
        errorMessage="Indexing failed"
        onDismiss={() => {}}
      />,
    );

    expect(html).toContain('Indexing failed');
    expect(html).toContain('aria-label="Copy error"');
    expect(html).toContain('aria-label="Expand error"');
    expect(html).toContain('aria-label="Dismiss"');
  });
});
