import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';
import { StaticBubble } from '../components/chat-panel/chat-box/StaticBubble';
import { PlanTaskItem } from '../components/chat-panel/chat-box/tool-renderers/PlanRenderer';
import {
  shouldDisplayStreamingAgent,
  shouldDisplayStreamingContentAgent,
} from '../hooks/chatStreamTypes';

describe('Plan tool rendering', () => {
  it('renders a terminal provider error notice inside the assistant bubble', () => {
    const html = renderToStaticMarkup(
      <StaticBubble
        message={{
          id: 'error-run-1',
          role: 'assistant',
          content: '',
          timestamp: '2026-04-24T00:00:01.000Z',
          tool_calls: [],
          metadata: {
            stream_error: 'LLM error: server error from DeepSeek-V4: HTTP 400 Bad Request: read timeout',
          },
        }}
      />,
    );

    expect(html).toContain('Provider error');
    expect(html).toContain('LLM error: server error from DeepSeek-V4');
  });

  it('renders plan-writer output with tasks as structured task content', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="success"
        result="1 tasks extracted"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            plan_content: '',
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Render structured plan output',
                description: 'Use structured metadata instead of raw JSON strings.',
                depends_on: [],
                status: 'pending',
                estimated_iterations: 12,
                key_files: [
                  'crates/y-gui/src/components/chat-panel/chat-box/ToolCallCard.tsx',
                ],
                acceptance_criteria: [
                  'Task results render as a dedicated component',
                ],
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Tasks');
    expect(html).toContain('Render structured plan output');
    expect(html).not.toContain('plan_title');
  });

  it('renders plan summary fields as a readable review component', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-review-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Improve GUI Plan review flow' }),
        }}
        status="success"
        result="2 tasks extracted"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            stage_status: 'completed',
            review_status: 'approved',
            plan_title: 'GUI Plan Review Flow',
            plan_file: '/tmp/gui-plan-review.md',
            estimated_effort: 'Short(1-4h)',
            overview: 'Render a readable plan and pause for human review before execution.',
            scope_in: ['Readable plan summary', 'Human review gate'],
            scope_out: ['Special-case handling for user feedback'],
            guardrails: ['Do not execute phases before review'],
            tasks: [
              {
                id: 'phase-1',
                phase: 1,
                title: 'Render reviewable plan',
                description: 'Show overview, scope, guardrails, and task cards.',
                depends_on: [],
                status: 'pending',
                estimated_iterations: 12,
                key_files: [
                  'crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx',
                ],
                acceptance_criteria: ['Plan metadata is readable without raw JSON'],
              },
              {
                id: 'phase-2',
                phase: 2,
                title: 'Pause before execution',
                description: 'Wait for explicit user review before phase execution.',
                depends_on: ['phase-1'],
                status: 'pending',
                estimated_iterations: 12,
                key_files: ['crates/y-service/src/plan_orchestrator.rs'],
                acceptance_criteria: ['Phase executor does not start before review'],
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Approved');
    expect(html).toContain('Short(1-4h)');
    expect(html).toContain('Render a readable plan');
    expect(html).toContain('Readable plan summary');
    expect(html).toContain('Do not execute phases before review');
    expect(html).toContain('2 phases');
    expect(html).not.toContain('scope_in');
  });

  it('renders plan-writer output as markdown instead of a preformatted block', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-writer-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="success"
        result="Plan written"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            plan_content: '# Implementation Plan\n\n- Render markdown\n- Keep task status updated',
          },
        }}
      />,
    );

    expect(html).toContain('<h1>Implementation Plan</h1>');
    expect(html).toContain('<li>Render markdown</li>');
    expect(html).not.toContain('<pre');
  });

  it('renders an in-progress plan stage as running before sub-agent tool results finish', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-start-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="success"
        result="Starting plan generation"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            stage_status: 'running',
            plan_title: '',
            plan_file: '/tmp/gui-plan.md',
            plan_content: '',
          },
        }}
      />,
    );

    expect(html).toContain('Running...');
    expect(html).toContain('Fix GUI Plan stream rendering');
  });

  it('renders the writer-stage card as Running while awaiting user review', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-awaiting-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan review status' }),
        }}
        status="success"
        result="2 tasks extracted"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            stage_status: 'completed',
            review_status: 'awaiting_user',
            plan_title: 'GUI Plan Review Status',
            plan_file: '/tmp/gui-plan.md',
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'First',
                description: '',
                depends_on: [],
                status: 'pending',
                estimated_iterations: 4,
                key_files: [],
                acceptance_criteria: [],
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Running...');
    expect(html).toContain('Awaiting review');
    expect(html).not.toContain('>Done<');
  });

  it('does not show plan_execution as Running once the tool call has terminated', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-exec-cancelled-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Cancellation stuck-on-running' }),
        }}
        status="error"
        result="Cancelled"
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Cancelled Plan',
            plan_file: '/tmp/cancelled.md',
            plan_run_id: 'run-cancelled-1',
            total_phases: 3,
            completed: 1,
            failed: 0,
            tasks: [
              {
                id: 'task-1', phase: 1, title: 'First', description: '', depends_on: [],
                status: 'completed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-2', phase: 2, title: 'Second', description: '', depends_on: [],
                status: 'in_progress', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-3', phase: 3, title: 'Third', description: '', depends_on: [],
                status: 'pending', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
            ],
            phases: [],
          },
        }}
      />,
    );

    expect(html).not.toContain('Running...');
    expect(html).toContain('tool-status-error');
  });

  it('keeps the execution card Running while phases are in progress on success-state progress events', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-exec-live-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Live execution running indicator' }),
        }}
        status="success"
        result="Execution in progress"
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Live Exec',
            plan_file: '/tmp/live.md',
            plan_run_id: 'run-live-1',
            total_phases: 2,
            completed: 0,
            failed: 0,
            tasks: [
              {
                id: 'task-1', phase: 1, title: 'First', description: '', depends_on: [],
                status: 'in_progress', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-2', phase: 2, title: 'Second', description: '', depends_on: ['task-1'],
                status: 'pending', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
            ],
            phases: [],
          },
        }}
      />,
    );

    expect(html).toContain('Running...');
    expect(html).not.toContain('>Done<');
  });

  it('renders a structured plan provider error without falling back to raw JSON', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-error-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="error"
        result="LLM error: server error from DeepSeek-V4: HTTP 400 Bad Request: read timeout"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            stage_status: 'failed',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            plan_content: '# Implementation Plan',
            tasks: [],
          },
        }}
      />,
    );

    expect(html).toContain('Failed');
    expect(html).toContain('GUI Plan Stream Fix');
    expect(html).toContain('LLM error: server error from DeepSeek-V4');
    expect(html).toContain('<h1>Implementation Plan</h1>');
    expect(html).not.toContain('plan_title');
  });

  it('renders per-phase task statuses for plan execution', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-exec-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="success"
        result="Execution in progress"
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            total_phases: 2,
            completed: 1,
            failed: 0,
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Render markdown output',
                description: 'Use markdown rendering for plan output.',
                depends_on: [],
                status: 'completed',
                estimated_iterations: 4,
                key_files: ['crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx'],
                acceptance_criteria: ['Plan content renders as markdown'],
              },
              {
                id: 'task-2',
                phase: 2,
                title: 'Keep execution state visible',
                description: 'Do not drop the running indicator during long plan execution.',
                depends_on: ['task-1'],
                status: 'in_progress',
                estimated_iterations: 6,
                key_files: ['crates/y-gui/src/hooks/useChat.ts'],
                acceptance_criteria: ['Stop button stays visible while run is active'],
              },
            ],
            phases: [
              {
                task_id: 'task-1',
                phase: 1,
                title: 'Render markdown output',
                status: 'completed',
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Completed');
    expect(html).toContain('In Progress');
  });

  it('does not render verbose phase summaries when plan execution fails', () => {
    const verboseSummary = 'verbose completed phase output '.repeat(80);
    const result = JSON.stringify({
      plan_title: 'GUI Plan Stream Fix',
      plan_file: '/tmp/gui-plan.md',
      plan_run_id: 'plan-run-1',
      total_phases: 8,
      completed: 7,
      failed: 1,
      phases: [
        {
          task_id: 'task-1',
          phase: 1,
          title: 'Completed phase',
          status: 'completed',
          summary: verboseSummary,
        },
        {
          task_id: 'task-8',
          phase: 8,
          title: 'Failing phase',
          status: 'failed',
          error: 'Phase failed while running tests',
        },
      ],
    });

    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-exec-failed-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
        }}
        status="error"
        result={result}
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            plan_run_id: 'plan-run-1',
            total_phases: 8,
            completed: 7,
            failed: 1,
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Completed phase',
                description: '',
                depends_on: [],
                status: 'completed',
                estimated_iterations: 4,
                key_files: [],
                acceptance_criteria: [],
              },
              {
                id: 'task-8',
                phase: 8,
                title: 'Failing phase',
                description: '',
                depends_on: ['task-1'],
                status: 'failed',
                estimated_iterations: 4,
                key_files: [],
                acceptance_criteria: [],
              },
            ],
            phases: [
              {
                task_id: 'task-1',
                phase: 1,
                title: 'Completed phase',
                status: 'completed',
                summary: verboseSummary,
              },
              {
                task_id: 'task-8',
                phase: 8,
                title: 'Failing phase',
                status: 'failed',
                error: 'Phase failed while running tests',
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('7/8 completed');
    expect(html).toContain('1 failed');
    expect(html).toContain('Failing phase');
    expect(html).not.toContain('verbose completed phase output');
    expect(html).not.toContain('&quot;phases&quot;');
  });

  it('keeps execute-task details collapsed by default', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-exec-collapsed-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Improve execute task rendering' }),
        }}
        status="success"
        result="Execution in progress"
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'GUI Execute Tasklist',
            plan_file: '/tmp/gui-plan.md',
            total_phases: 1,
            completed: 0,
            failed: 0,
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Refine execute task row',
                description: 'First line of detail.\nSecond line should stay visible after expand.',
                depends_on: [],
                status: 'in_progress',
                estimated_iterations: 2,
                key_files: ['crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx'],
                acceptance_criteria: ['Task detail stays collapsed by default'],
              },
            ],
            phases: [],
          },
        }}
      />,
    );

    expect(html).toContain('Refine execute task row');
    expect(html).toContain('In Progress');
    expect(html).toContain('tool-call-plan-task-toggle');
    expect(html).toContain('tool-call-plan-task-status-column');
    expect(html).not.toContain('First line of detail.');
    expect(html).not.toContain('Task detail stays collapsed by default');
    expect(html).not.toContain('PlanRenderer.tsx');
  });

  it('preserves line breaks when an execute-task detail is expanded', () => {
    const html = renderToStaticMarkup(
      <PlanTaskItem
        task={{
          id: 'task-expanded-1',
          phase: 1,
          title: 'Preserve multiline descriptions',
          description: 'First line of detail.\nSecond line should stay visible after expand.',
          dependsOn: [],
          status: 'pending',
          estimatedIterations: 1,
          keyFiles: ['crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx'],
          acceptanceCriteria: ['Expanded task detail keeps original newlines'],
        }}
        defaultExpanded
      />,
    );

    expect(html).toContain('First line of detail.');
    expect(html).toContain('Second line should stay visible after expand.');
    expect(html).toContain('<br/>');
    expect(html).toContain('tool-call-plan-task-detail');
  });

  it('shows dependency info in expanded task detail', () => {
    const html = renderToStaticMarkup(
      <PlanTaskItem
        task={{
          id: 'phase-3',
          phase: 3,
          title: 'Integrate auth with billing',
          description: 'Wire up billing API to use auth tokens.',
          dependsOn: ['phase-1', 'phase-2'],
          status: 'pending',
          estimatedIterations: 8,
          keyFiles: [],
          acceptanceCriteria: [],
        }}
        defaultExpanded
      />,
    );

    expect(html).toContain('Deps');
    expect(html).toContain('phase-1, phase-2');
  });

  it('shows Independent label for tasks with no dependencies', () => {
    const html = renderToStaticMarkup(
      <PlanTaskItem
        task={{
          id: 'phase-1',
          phase: 1,
          title: 'Set up database schema',
          description: 'Create initial tables.',
          dependsOn: [],
          status: 'pending',
          estimatedIterations: 5,
          keyFiles: [],
          acceptanceCriteria: [],
        }}
        defaultExpanded
      />,
    );

    expect(html).toContain('Independent');
  });

  it('shows parallel execution note when multiple tasks are in_progress', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-parallel-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Parallel execution test' }),
        }}
        status="success"
        result="Execution in progress"
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Parallel Test',
            plan_file: '/tmp/parallel.md',
            total_phases: 3,
            completed: 0,
            failed: 0,
            tasks: [
              {
                id: 'phase-1',
                phase: 1,
                title: 'Task A',
                description: '',
                depends_on: [],
                status: 'in_progress',
                estimated_iterations: 5,
                key_files: [],
                acceptance_criteria: [],
              },
              {
                id: 'phase-2',
                phase: 2,
                title: 'Task B',
                description: '',
                depends_on: [],
                status: 'in_progress',
                estimated_iterations: 5,
                key_files: [],
                acceptance_criteria: [],
              },
              {
                id: 'phase-3',
                phase: 3,
                title: 'Task C',
                description: '',
                depends_on: ['phase-1', 'phase-2'],
                status: 'pending',
                estimated_iterations: 5,
                key_files: [],
                acceptance_criteria: [],
              },
            ],
            phases: [],
          },
        }}
      />,
    );

    expect(html).toContain('2 tasks running in parallel');
    expect(html).toContain('tool-call-plan-parallel-note');
  });

  it('renders a partial-failure execution as Partial, not a hard Failed', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-partial-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Partial failure test' }),
        }}
        status="error"
        result={JSON.stringify({
          plan_title: 'Partial Test',
          total_phases: 3,
          completed: 2,
          failed: 1,
          phases: [],
        })}
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Partial Test',
            plan_file: '/tmp/partial.md',
            plan_run_id: 'plan-run-partial',
            total_phases: 3,
            completed: 2,
            failed: 1,
            tasks: [
              {
                id: 'task-1', phase: 1, title: 'Done A', description: '', depends_on: [],
                status: 'completed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-2', phase: 2, title: 'Done B', description: '', depends_on: [],
                status: 'completed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-3', phase: 3, title: 'Broken C', description: '', depends_on: [],
                status: 'failed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
            ],
            phases: [],
          },
        }}
      />,
    );

    expect(html).toContain('Partial');
    expect(html).toContain('1 failed');
    expect(html).toContain('tool-status-partial');
    expect(html).not.toContain('tool-status-error');
  });

  it('renders the per-phase error message for a failed phase', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-phase-error-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Phase error visibility' }),
        }}
        status="error"
        result={JSON.stringify({
          plan_title: 'Phase Error Test',
          total_phases: 1,
          completed: 0,
          failed: 1,
          phases: [],
        })}
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Phase Error Test',
            plan_file: '/tmp/phase-error.md',
            plan_run_id: 'plan-run-err',
            total_phases: 1,
            completed: 0,
            failed: 1,
            tasks: [
              {
                id: 'task-1', phase: 1, title: 'Compile crate', description: '', depends_on: [],
                status: 'failed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
            ],
            phases: [
              {
                task_id: 'task-1', phase: 1, title: 'Compile crate', status: 'failed',
                error: 'phase-1 execution failed: provider timed out after 3 retries',
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('tool-call-plan-task-error');
    expect(html).toContain('provider timed out after 3 retries');
  });

  it('renders a dependency-skipped phase as Skipped rather than Pending', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-skipped-1',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'Skipped phase labeling' }),
        }}
        status="error"
        result={JSON.stringify({ plan_title: 'Skip Test', total_phases: 2, completed: 0, failed: 1, phases: [] })}
        metadata={{
          display: {
            kind: 'plan_execution',
            plan_title: 'Skip Test',
            plan_file: '/tmp/skip.md',
            plan_run_id: 'plan-run-skip',
            total_phases: 2,
            completed: 0,
            failed: 1,
            tasks: [
              {
                id: 'task-1', phase: 1, title: 'Upstream', description: '', depends_on: [],
                status: 'failed', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
              {
                id: 'task-2', phase: 2, title: 'Downstream', description: '', depends_on: ['task-1'],
                status: 'skipped', estimated_iterations: 4, key_files: [], acceptance_criteria: [],
              },
            ],
            phases: [
              { task_id: 'task-2', phase: 2, title: 'Downstream', status: 'skipped', error: 'dependency failed' },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Skipped');
    expect(html).toContain('tool-call-plan-task-status--skipped');
  });

  it('renders a visible Retry-from-here action for terminal phases', () => {
    const html = renderToStaticMarkup(
      <PlanTaskItem
        task={{
          id: 'task-1',
          phase: 1,
          title: 'Failing phase',
          description: '',
          dependsOn: [],
          status: 'failed',
          estimatedIterations: 4,
          keyFiles: [],
          acceptanceCriteria: [],
          error: 'phase-1 execution failed: build error',
        }}
        defaultExpanded
        planRunId="plan-run-1"
        sessionId="session-1"
        onRetryFromHere={() => {}}
      />,
    );

    expect(html).toContain('Retry from here');
    expect(html).toContain('tool-call-plan-task-retry');
    expect(html).toContain('build error');
  });
});

describe('shouldDisplayStreamingAgent', () => {
  it('keeps root-agent deltas and hides sub-agent deltas', () => {
    expect(shouldDisplayStreamingAgent(undefined)).toBe(true);
    expect(shouldDisplayStreamingAgent('chat-turn')).toBe(true);
    expect(shouldDisplayStreamingAgent('plan-writer')).toBe(true);
  });

  it('hides plan-writer content deltas while keeping plan orchestrator cards visible', () => {
    expect(shouldDisplayStreamingContentAgent('chat-turn')).toBe(true);
    expect(shouldDisplayStreamingContentAgent('plan-orchestrator')).toBe(true);
    expect(shouldDisplayStreamingContentAgent('plan-writer')).toBe(false);
  });
});
