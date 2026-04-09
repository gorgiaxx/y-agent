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
});

describe('shouldDisplayStreamingAgent', () => {
  it('keeps root-agent deltas and hides sub-agent deltas', () => {
    expect(shouldDisplayStreamingAgent(undefined)).toBe(true);
    expect(shouldDisplayStreamingAgent('chat-turn')).toBe(true);
    expect(shouldDisplayStreamingAgent('task-decomposer')).toBe(false);
    expect(shouldDisplayStreamingAgent('plan-writer')).toBe(false);
  });
});
