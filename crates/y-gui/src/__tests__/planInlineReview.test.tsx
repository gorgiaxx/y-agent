import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { PlanReviewProvider } from '../components/chat-panel/PlanReviewContext';
import type { PlanReviewState } from '../components/chat-panel/planReviewState';
import { PlanReviewInline } from '../components/chat-panel/chat-box/tool-renderers/PlanReviewInline';
import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';

function makePlanReviewCtx(overrides: Partial<PlanReviewState> = {}): PlanReviewState {
  return {
    reviewId: 'review-1',
    onApprove: vi.fn(),
    onRevise: vi.fn(),
    onReject: vi.fn(),
    ...overrides,
  };
}

describe('PlanReviewInline', () => {
  it('renders textarea, Approve button, and Reject button', () => {
    const html = renderToStaticMarkup(
      <PlanReviewInline
        reviewId="review-1"
        onApprove={vi.fn()}
        onRevise={vi.fn()}
        onReject={vi.fn()}
      />,
    );

    expect(html).toContain('plan-review-inline-textarea');
    expect(html).toContain('Approve');
    expect(html).toContain('Reject');
    expect(html).not.toContain('Revise');
  });

  it('renders hint text for empty input (approve mode)', () => {
    const html = renderToStaticMarkup(
      <PlanReviewInline
        reviewId="review-1"
        onApprove={vi.fn()}
        onRevise={vi.fn()}
        onReject={vi.fn()}
      />,
    );

    expect(html).toContain('Enter = Approve');
  });
});

describe('PlanRenderer inline review integration', () => {
  it('shows inline review controls when reviewStatus is awaiting_user and context is provided', () => {
    const ctx = makePlanReviewCtx();
    const html = renderToStaticMarkup(
      <PlanReviewProvider value={ctx}>
        <ToolCallCard
          toolCall={{
            id: 'plan-1',
            name: 'Plan',
            arguments: JSON.stringify({ request: 'test plan' }),
          }}
          status="success"
          result="1 tasks extracted"
          metadata={{
            display: {
              kind: 'plan_stage',
              stage: 'plan_writer',
              plan_title: 'Test Plan',
              plan_file: '/tmp/plan.md',
              plan_content: '',
              review_status: 'awaiting_user',
              tasks: [
                {
                  id: 'task-1',
                  phase: 1,
                  title: 'Do the thing',
                  description: '',
                  depends_on: [],
                  status: 'pending',
                  estimated_iterations: 5,
                  key_files: [],
                  acceptance_criteria: [],
                },
              ],
            },
          }}
        />
      </PlanReviewProvider>,
    );

    expect(html).toContain('Awaiting review');
    expect(html).toContain('plan-review-inline');
    expect(html).toContain('Approve');
    expect(html).toContain('Reject');
  });

  it('does not show inline review controls when reviewStatus is approved', () => {
    const ctx = makePlanReviewCtx();
    const html = renderToStaticMarkup(
      <PlanReviewProvider value={ctx}>
        <ToolCallCard
          toolCall={{
            id: 'plan-2',
            name: 'Plan',
            arguments: JSON.stringify({ request: 'test plan' }),
          }}
          status="success"
          result="1 tasks extracted"
          metadata={{
            display: {
              kind: 'plan_stage',
              stage: 'plan_writer',
              plan_title: 'Test Plan',
              plan_file: '/tmp/plan.md',
              plan_content: '',
              review_status: 'approved',
              tasks: [
                {
                  id: 'task-1',
                  phase: 1,
                  title: 'Do the thing',
                  description: '',
                  depends_on: [],
                  status: 'pending',
                  estimated_iterations: 5,
                  key_files: [],
                  acceptance_criteria: [],
                },
              ],
            },
          }}
        />
      </PlanReviewProvider>,
    );

    expect(html).toContain('Approved');
    expect(html).not.toContain('plan-review-inline');
  });

  it('does not show inline review controls when no PlanReviewContext is provided', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'plan-3',
          name: 'Plan',
          arguments: JSON.stringify({ request: 'test plan' }),
        }}
        status="success"
        result="1 tasks extracted"
        metadata={{
          display: {
            kind: 'plan_stage',
            stage: 'plan_writer',
            plan_title: 'Test Plan',
            plan_file: '/tmp/plan.md',
            plan_content: '',
            review_status: 'awaiting_user',
            tasks: [
              {
                id: 'task-1',
                phase: 1,
                title: 'Do the thing',
                description: '',
                depends_on: [],
                status: 'pending',
                estimated_iterations: 5,
                key_files: [],
                acceptance_criteria: [],
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('Awaiting review');
    expect(html).not.toContain('plan-review-inline');
  });

  it('shows feedback_received badge for revision status', () => {
    const ctx = makePlanReviewCtx({ reviewId: null });
    const html = renderToStaticMarkup(
      <PlanReviewProvider value={ctx}>
        <ToolCallCard
          toolCall={{
            id: 'plan-4',
            name: 'Plan',
            arguments: JSON.stringify({ request: 'test plan' }),
          }}
          status="success"
          result="1 tasks extracted"
          metadata={{
            display: {
              kind: 'plan_stage',
              stage: 'plan_writer',
              plan_title: 'Revised Plan',
              plan_file: '/tmp/plan.md',
              plan_content: '',
              review_status: 'feedback_received',
              review_feedback: 'Reduce scope to auth module only',
              tasks: [
                {
                  id: 'task-1',
                  phase: 1,
                  title: 'Auth module only',
                  description: '',
                  depends_on: [],
                  status: 'pending',
                  estimated_iterations: 3,
                  key_files: [],
                  acceptance_criteria: [],
                },
              ],
            },
          }}
        />
      </PlanReviewProvider>,
    );

    expect(html).toContain('Feedback received');
    expect(html).toContain('Reduce scope to auth module only');
  });
});
