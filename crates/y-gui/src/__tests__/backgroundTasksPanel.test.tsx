import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import {
  BackgroundTasksSidebarNav,
  BackgroundTasksOutputPanel,
  BackgroundTasksSidebarPanel,
} from '../components/background-tasks/BackgroundTasksPanel';
import type { BackgroundTaskLogEntry } from '../hooks/useBackgroundTasks';
import type { BackgroundTaskInfo } from '../hooks/useBackgroundTasks';

const noop = () => {};

const tasks: BackgroundTaskInfo[] = [
  {
    process_id: 'proc-1',
    backend: 'native',
    command: 'npm run dev',
    working_dir: '/repo/app',
    status: 'running',
    exit_code: null,
    error: null,
    duration_ms: 12_400,
  },
  {
    process_id: 'proc-2',
    backend: 'native',
    command: 'cargo run --bin api',
    working_dir: null,
    status: 'failed',
    exit_code: null,
    error: 'port already in use',
    duration_ms: 3_200,
  },
];

const logs: Record<string, BackgroundTaskLogEntry[]> = {
  'proc-1': [
    {
      id: 'proc-1-output-1',
      stream: 'stdout',
      content: '\u001b[31mVITE ready\u001b[0m in 340 ms\nDownloading 10%\rDownloading 20%\n',
      timestamp: 2,
    },
  ],
  'proc-2': [
    {
      id: 'proc-2-stderr-1',
      stream: 'stderr',
      content: 'error: address already in use',
      timestamp: 3,
    },
  ],
};

describe('BackgroundTasksPanel', () => {
  it('renders background tasks as a secondary sidebar with a back action', () => {
    const html = renderToStaticMarkup(
      <BackgroundTasksSidebarNav onBack={noop}>
        <BackgroundTasksSidebarPanel
          tasks={tasks}
          loading={false}
          error={null}
          selectedProcessId="proc-1"
          onSelectTask={noop}
          onRefresh={noop}
        />
      </BackgroundTasksSidebarNav>,
    );

    expect(html).toContain('Back');
    expect(html).toContain('Background tasks');
    expect(html).toContain('2 tasks');
  });

  it('renders task navigation as a sidebar list', () => {
    const html = renderToStaticMarkup(
      <BackgroundTasksSidebarPanel
        tasks={tasks}
        loading={false}
        error={null}
        selectedProcessId="proc-1"
        onSelectTask={noop}
        onRefresh={noop}
      />,
    );

    expect(html).toContain('Background tasks');
    expect(html).toContain('2 tasks');
    expect(html).toContain('npm run dev');
    expect(html).toContain('active');
    expect(html).not.toContain('background-tasks-icon-btn');
    expect(html).not.toContain('background-task-count');
    expect(html).not.toContain('sidebar-item-badge');
  });

  it('renders output without stdin controls or stream labels and preserves ansi color', () => {
    const html = renderToStaticMarkup(
      <BackgroundTasksOutputPanel
        task={tasks[0]}
        logs={logs['proc-1']}
        busy={false}
        onPoll={noop}
        onKill={noop}
      />,
    );

    expect(html).toContain('npm run dev');
    expect(html).toContain('VITE ready');
    expect(html).toContain('ansi-fg-red');
    expect(html).toContain('Downloading 20%');
    expect(html).not.toContain('Downloading 10%');
    expect(html).not.toContain('\u001b');
    expect(html).not.toContain('stdin');
    expect(html).not.toContain('Send stdin');
    expect(html).not.toContain('stdout');
  });

  it('renders task detail controls when expanded in the main view', () => {
    const html = renderToStaticMarkup(
      <BackgroundTasksOutputPanel
        task={tasks[1]}
        logs={logs['proc-2']}
        busy={false}
        onPoll={noop}
        onKill={noop}
      />,
    );

    expect(html).toContain('cargo run --bin api');
    expect(html).toContain('error: address already in use');
    expect(html).toContain('aria-label="Poll task proc-2"');
    expect(html).not.toContain('background-tasks-icon-btn');
    expect(html).not.toContain('npm run dev');
  });
});
