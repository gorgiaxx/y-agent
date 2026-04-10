import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';
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
});

describe('shouldDisplayStreamingAgent', () => {
  it('keeps root-agent deltas and hides sub-agent deltas', () => {
    expect(shouldDisplayStreamingAgent(undefined)).toBe(true);
    expect(shouldDisplayStreamingAgent('chat-turn')).toBe(true);
    expect(shouldDisplayStreamingAgent('task-decomposer')).toBe(false);
    expect(shouldDisplayStreamingAgent('plan-writer')).toBe(false);
  });
});
