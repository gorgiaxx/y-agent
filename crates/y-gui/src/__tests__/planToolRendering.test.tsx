import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';
import { PlanTaskItem } from '../components/chat-panel/chat-box/tool-renderers/PlanRenderer';
import { shouldDisplayStreamingAgent } from '../hooks/chatStreamTypes';

describe('Plan tool rendering', () => {
  it('renders task-decomposer output as structured task content', () => {
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
            stage: 'task_decomposer',
            plan_title: 'GUI Plan Stream Fix',
            plan_file: '/tmp/gui-plan.md',
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Render task decomposer output',
                description: 'Use structured metadata instead of raw JSON strings.',
                depends_on: [],
                status: 'pending',
                estimated_iterations: 12,
                key_files: [
                  'crates/y-gui/src/components/chat-panel/chat-box/ToolCallCard.tsx',
                ],
                acceptance_criteria: [
                  'Task decomposer results render as a dedicated component',
                ],
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Tasks');
    expect(html).toContain('Render task decomposer output');
    expect(html).not.toContain('plan_title');
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
});

describe('shouldDisplayStreamingAgent', () => {
  it('keeps root-agent deltas and hides sub-agent deltas', () => {
    expect(shouldDisplayStreamingAgent(undefined)).toBe(true);
    expect(shouldDisplayStreamingAgent('chat-turn')).toBe(true);
    expect(shouldDisplayStreamingAgent('task-decomposer')).toBe(false);
    expect(shouldDisplayStreamingAgent('plan-writer')).toBe(true);
  });
});
