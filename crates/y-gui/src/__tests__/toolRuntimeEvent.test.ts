import { describe, expect, it } from 'vitest';

import { snapshotFromToolRuntimeEvent } from '../hooks/useBackgroundTasks';

describe('tool runtime background-task mapping', () => {
  it('maps stdout chunks into incremental running snapshots', () => {
    expect(snapshotFromToolRuntimeEvent({
      session_id: 'session-1',
      task_id: 'process-1',
      tool_name: 'ShellExec',
      backend: 'native',
      occurred_at: '2026-07-17T00:00:00Z',
      type: 'output_chunk',
      stream: 'stdout',
      content: 'ready\n',
    })).toEqual({
      process_id: 'process-1',
      backend: 'native',
      status: 'running',
      exit_code: null,
      error: null,
      stdout: 'ready\n',
      stderr: '',
      duration_ms: 0,
    });
  });

  it('maps process completion into a terminal snapshot', () => {
    expect(snapshotFromToolRuntimeEvent({
      session_id: 'session-1',
      task_id: 'process-1',
      tool_name: 'ShellExec',
      backend: 'native',
      occurred_at: '2026-07-17T00:00:00Z',
      type: 'process_completed',
      exit_code: 0,
      duration_ms: 125,
    })).toMatchObject({
      process_id: 'process-1',
      status: 'completed',
      exit_code: 0,
      duration_ms: 125,
    });
  });
});
