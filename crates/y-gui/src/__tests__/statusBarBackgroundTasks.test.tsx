import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

import { StatusBar } from '../components/chat-panel/StatusBar';

vi.mock('../components/common/ProviderIconPicker', () => ({
  ProviderIconImg: () => null,
}));

describe('StatusBar background task entry', () => {
  it('renders a plain task status button for opening the tasks view', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        backgroundTasks={{
          total: 2,
          running: 1,
          failed: 0,
          onClick: () => {},
        }}
      />,
    );

    expect(html).toContain('1 running');
    expect(html).toContain('2 tasks');
    expect(html).toContain('Open background tasks');
    expect(html).toContain('status-background-tasks--running');
    expect(html).not.toContain('background-task-count');
    expect(html).not.toContain('sidebar-item-badge');
  });

  it('keeps the task entry visible when there are no tasks', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        backgroundTasks={{
          total: 0,
          running: 0,
          failed: 0,
          onClick: () => {},
        }}
      />,
    );

    expect(html).toContain('0 tasks');
    expect(html).toContain('Open background tasks');
  });
});
