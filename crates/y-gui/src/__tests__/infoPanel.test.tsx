import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ReactNode } from 'react';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
}));

const mockPlatform = vi.hoisted(() => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
  revealInFileManager: vi.fn().mockResolvedValue(undefined),
  capabilities: {
    revealFileManager: true,
  },
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(async () => []),
    listen: vi.fn(async () => () => {}),
  },
  platform: {
    capabilities: mockPlatform.capabilities,
    openUrl: mockPlatform.openUrl,
    revealInFileManager: mockPlatform.revealInFileManager,
  },
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}));

vi.mock('../components/ui', async () => {
  const React = await import('react');
  return {
    Button: ({ children, ...props }: { children?: ReactNode } & Record<string, unknown>) =>
      React.createElement('button', props, children),
  };
});

import { InfoPanel } from '../components/observation/InfoPanel';
import type { ModifiedFileEntry } from '../hooks/useInfoPanel';
import type { PlanWriterStageDisplay, PlanExecutionDisplay } from '../components/chat-panel/chat-box/planToolDisplay';
import type { LoopRoundStageDisplay } from '../components/chat-panel/chat-box/toolCallUtils';

describe('InfoPanel', () => {
  const noopFn = () => {};

  it('renders empty state when no data is provided', () => {
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('No activity yet');
    expect(html).toContain('info-empty');
  });

  it('renders modified files section with file cards', () => {
    const files: ModifiedFileEntry[] = [
      { filePath: '/src/main.rs', toolType: 'edit', displayName: 'main.rs', count: 3, diffs: [] },
      { filePath: '/src/lib.rs', toolType: 'write', displayName: 'lib.rs', count: 1, diffs: [] },
    ];
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={files}
        plans={[]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('Modified Files');
    expect(html).toContain('main.rs');
    expect(html).toContain('lib.rs');
    expect(html).toContain('3x');
    expect(html).not.toContain('1x');
  });

  it('renders all accumulated diffs for a modified file', () => {
    const files: ModifiedFileEntry[] = [
      {
        filePath: '/src/main.rs',
        toolType: 'edit',
        displayName: 'main.rs',
        count: 2,
        diffs: [
          { oldString: 'const first = 1;\n', newString: 'const first = 2;\n' },
          { oldString: 'const second = 1;\n', newString: 'const second = 2;\n' },
        ],
      },
    ];

    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={files}
        plans={[]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );

    expect(html).toContain('Diff 1');
    expect(html).toContain('Diff 2');
    expect(html).toContain('const first = 2;');
    expect(html).toContain('const second = 2;');
  });

  it('marks modified file cards as context menu targets', () => {
    const files: ModifiedFileEntry[] = [
      { filePath: '/src/main.rs', toolType: 'edit', displayName: 'main.rs', count: 1, diffs: [] },
    ];

    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={files}
        plans={[]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );

    expect(html).toContain('data-file-context-menu="true"');
  });

  it('renders plan section with tasks and progress', () => {
    const plan: PlanWriterStageDisplay = {
      kind: 'plan_stage',
      stage: 'plan_writer',
      stageStatus: 'running',
      planTitle: 'Refactor auth',
      planFile: 'plan.md',
      planContent: '',
      estimatedEffort: '2h',
      overview: '',
      scopeIn: [],
      scopeOut: [],
      guardrails: [],
      reviewStatus: '',
      reviewFeedback: '',
      reviewId: '',
      tasks: [
        {
          id: 't1', phase: 1, title: 'Add tests', description: '',
          dependsOn: [], status: 'completed', estimatedIterations: 1,
          keyFiles: [], acceptanceCriteria: [],
        },
        {
          id: 't2', phase: 2, title: 'Implement feature', description: '',
          dependsOn: [], status: 'in_progress', estimatedIterations: 2,
          keyFiles: [], acceptanceCriteria: [],
        },
      ],
    };
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[plan]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('Plan');
    expect(html).toContain('Refactor auth');
    expect(html).toContain('Add tests');
    expect(html).toContain('Implement feature');
    expect(html).toContain('info-progress-fill');
  });

  it('does not show plan_execution as running when remaining phases are skipped', () => {
    const plan: PlanExecutionDisplay = {
      kind: 'plan_execution',
      planTitle: 'Mixed outcome plan',
      planFile: 'plan.md',
      planRunId: 'run-skipped-1',
      totalPhases: 3,
      completed: 2,
      failed: 0,
      planRunStatus: '',
      tasks: [
        {
          id: 't1', phase: 1, title: 'Done A', description: '',
          dependsOn: [], status: 'completed', estimatedIterations: 1,
          keyFiles: [], acceptanceCriteria: [],
        },
        {
          id: 't2', phase: 2, title: 'Done B', description: '',
          dependsOn: [], status: 'completed', estimatedIterations: 1,
          keyFiles: [], acceptanceCriteria: [],
        },
        {
          id: 't3', phase: 3, title: 'Skipped', description: '',
          dependsOn: ['t2'], status: 'skipped', estimatedIterations: 1,
          keyFiles: [], acceptanceCriteria: [],
        },
      ],
      phases: [],
    };
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[plan]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).not.toContain('status-running');
  });

  it('does not show plan_stage as completed while it is awaiting user review', () => {
    const plan: PlanWriterStageDisplay = {
      kind: 'plan_stage',
      stage: 'plan_writer',
      stageStatus: 'completed',
      planTitle: 'Awaiting plan',
      planFile: 'plan.md',
      planContent: '',
      estimatedEffort: '',
      overview: '',
      scopeIn: [],
      scopeOut: [],
      guardrails: [],
      reviewStatus: 'awaiting_user',
      reviewFeedback: '',
      reviewId: '',
      tasks: [
        {
          id: 't1', phase: 1, title: 'Pending task', description: '',
          dependsOn: [], status: 'pending', estimatedIterations: 1,
          keyFiles: [], acceptanceCriteria: [],
        },
      ],
    };
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[plan]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('status-running');
    expect(html).not.toContain('status-completed');
  });

  it('renders loop section with round info', () => {
    const loop: LoopRoundStageDisplay = {
      kind: 'loop_round',
      round: 2,
      maxRounds: 5,
      roundStatus: 'executing',
      tasksCompleted: ['Fix lint'],
      tasksRemaining: ['Add docs'],
      converged: false,
      rounds: [],
    };
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[]}
        loopStatus={loop}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('Loop');
    expect(html).toContain('Round 2 / 5');
    expect(html).toContain('Fix lint');
    expect(html).toContain('Add docs');
  });

  it('renders backdrop wrapper in expanded mode', () => {
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[]}
        loopStatus={null}
        expanded={true}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).toContain('info-backdrop');
    expect(html).toContain('info-expanded');
  });

  it('does not render backdrop when not expanded', () => {
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        plans={[]}
        loopStatus={null}
        expanded={false}
        onToggleExpand={noopFn}
        onClose={noopFn}
      />,
    );
    expect(html).not.toContain('info-backdrop');
    expect(html).not.toContain('info-expanded');
  });
});
