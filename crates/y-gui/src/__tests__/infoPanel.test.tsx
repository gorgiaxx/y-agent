import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ReactNode } from 'react';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(async () => []),
    listen: vi.fn(async () => () => {}),
  },
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
import type { PlanWriterStageDisplay } from '../components/chat-panel/chat-box/planToolDisplay';
import type { LoopRoundStageDisplay } from '../components/chat-panel/chat-box/toolCallUtils';

describe('InfoPanel', () => {
  const noopFn = () => {};

  it('renders empty state when no data is provided', () => {
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={[]}
        planStatus={null}
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
      { filePath: '/src/main.rs', toolType: 'edit', displayName: 'main.rs', count: 3 },
      { filePath: '/src/lib.rs', toolType: 'write', displayName: 'lib.rs', count: 1 },
    ];
    const html = renderToStaticMarkup(
      <InfoPanel
        modifiedFiles={files}
        planStatus={null}
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
        planStatus={plan}
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
        planStatus={null}
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
        planStatus={null}
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
        planStatus={null}
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
