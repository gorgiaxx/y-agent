import { describe, expect, it } from 'vitest';
import {
  upsertToolResultRecord,
  upsertToolResultSegment,
} from '../hooks/toolResultUpdates';

const ASK_ARGS = JSON.stringify({
  questions: [
    {
      question: 'Which library?',
      options: ['React', 'Vue'],
    },
  ],
});

function makeAskUserResult(status: 'pending' | 'answered') {
  return {
    name: 'AskUser',
    arguments: ASK_ARGS,
    success: true,
    durationMs: status === 'pending' ? 12 : 128,
    resultPreview: JSON.stringify(
      status === 'pending'
        ? {
            status: 'pending',
            questions: [
              {
                question: 'Which library?',
                options: ['React', 'Vue'],
              },
            ],
          }
        : {
            status: 'answered',
            questions: [
              {
                question: 'Which library?',
                options: ['React', 'Vue'],
              },
            ],
            answers: {
              'Which library?': 'React',
            },
          },
    ),
  };
}

describe('toolResultUpdates', () => {
  it('replaces a pending AskUser result with the answered result', () => {
    const pending = makeAskUserResult('pending');
    const answered = makeAskUserResult('answered');

    const updated = upsertToolResultRecord([pending], answered);

    expect(updated.replacedIndex).toBe(0);
    expect(updated.records).toHaveLength(1);
    expect(updated.records[0]).toEqual(answered);
  });

  it('replaces the existing AskUser tool_result segment instead of appending', () => {
    const pending = makeAskUserResult('pending');
    const answered = makeAskUserResult('answered');

    const updated = upsertToolResultSegment(
      [
        { type: 'text', text: 'Need your input.' },
        { type: 'tool_result', record: pending },
      ],
      answered,
    );

    expect(updated.replacedIndex).toBe(1);
    expect(updated.segments).toHaveLength(2);
    expect(updated.segments[1]).toEqual({ type: 'tool_result', record: answered });
  });

  it('appends non-AskUser results normally', () => {
    const first = {
      name: 'Browser',
      arguments: JSON.stringify({ action: 'navigate', url: 'https://example.com' }),
      success: true,
      durationMs: 10,
      resultPreview: JSON.stringify({ status: 'ok', url: 'https://example.com' }),
    };
    const second = {
      ...first,
      resultPreview: JSON.stringify({ status: 'ok', url: 'https://example.org' }),
    };

    const updated = upsertToolResultRecord([first], second);

    expect(updated.replacedIndex).toBeNull();
    expect(updated.records).toHaveLength(2);
    expect(updated.records[1]).toEqual(second);
  });

  it('replaces the latest plan execution progress update for the same plan file', () => {
    const taskDecomposer = {
      name: 'Plan',
      arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
      success: true,
      durationMs: 8,
      resultPreview: '2 tasks extracted',
      metadata: {
        display: {
          kind: 'plan_stage',
          stage: 'task_decomposer',
          plan_title: 'GUI Plan Stream Fix',
          plan_file: '/tmp/gui-plan.md',
          tasks: [],
        },
      },
    };

    const firstExecution = {
      name: 'Plan',
      arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
      success: true,
      durationMs: 15,
      resultPreview: 'Phase 1 completed',
      metadata: {
        display: {
          kind: 'plan_execution',
          plan_title: 'GUI Plan Stream Fix',
          plan_file: '/tmp/gui-plan.md',
          total_phases: 2,
          completed: 1,
          failed: 0,
          tasks: [],
          phases: [{ task_id: 'task-1', status: 'completed' }],
        },
      },
    };

    const secondExecution = {
      ...firstExecution,
      durationMs: 30,
      resultPreview: 'Phase 2 completed',
      metadata: {
        display: {
          kind: 'plan_execution',
          plan_title: 'GUI Plan Stream Fix',
          plan_file: '/tmp/gui-plan.md',
          total_phases: 2,
          completed: 2,
          failed: 0,
          tasks: [],
          phases: [
            { task_id: 'task-1', status: 'completed' },
            { task_id: 'task-2', status: 'completed' },
          ],
        },
      },
    };

    const updated = upsertToolResultRecord(
      [taskDecomposer, firstExecution],
      secondExecution,
    );

    expect(updated.replacedIndex).toBe(1);
    expect(updated.records).toHaveLength(2);
    expect(updated.records[1]).toEqual(secondExecution);
  });

  it('replaces a task decomposer stage with plan execution progress for the same plan', () => {
    const taskDecomposer = {
      name: 'Plan',
      arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
      success: true,
      durationMs: 8,
      resultPreview: '2 tasks extracted',
      metadata: {
        display: {
          kind: 'plan_stage',
          stage: 'task_decomposer',
          plan_title: 'GUI Plan Stream Fix',
          plan_file: '/tmp/gui-plan.md',
          tasks: [],
        },
      },
    };

    const execution = {
      name: 'Plan',
      arguments: JSON.stringify({ request: 'Fix GUI Plan stream rendering' }),
      success: true,
      durationMs: 15,
      resultPreview: 'Phase 1 completed',
      metadata: {
        display: {
          kind: 'plan_execution',
          plan_title: 'GUI Plan Stream Fix',
          plan_file: '/tmp/gui-plan.md',
          total_phases: 2,
          completed: 1,
          failed: 0,
          tasks: [],
          phases: [{ task_id: 'task-1', status: 'completed' }],
        },
      },
    };

    const updatedRecords = upsertToolResultRecord([taskDecomposer], execution);

    expect(updatedRecords.replacedIndex).toBe(0);
    expect(updatedRecords.records).toHaveLength(1);
    expect(updatedRecords.records[0]).toEqual(execution);

    const updatedSegments = upsertToolResultSegment(
      [{ type: 'tool_result', record: taskDecomposer }],
      execution,
    );

    expect(updatedSegments.replacedIndex).toBe(0);
    expect(updatedSegments.segments).toHaveLength(1);
    expect(updatedSegments.segments[0]).toEqual({ type: 'tool_result', record: execution });
  });
});
