import { useEffect, useRef, useState } from 'react';
import { Check, ClipboardList, X } from 'lucide-react';
import { PlanTaskItem } from '../chat-box/tool-renderers/PlanRenderer';
import type { PlanTaskDisplay } from '../chat-box/toolCallUtils';
import './PlanReviewDialog.css';

interface PlanReviewDialogProps {
  reviewId: string;
  plan: Record<string, unknown>;
  onApprove: (reviewId: string) => void;
  onReject: (reviewId: string, feedback: string) => void;
}

function extractPlanFields(plan: Record<string, unknown>) {
  const planTitle = (plan.plan_title as string) || 'Untitled Plan';
  const estimatedEffort = (plan.estimated_effort as string) || '';
  const overview = (plan.overview as string) || '';
  const scopeIn = (plan.scope_in as string[]) || [];
  const scopeOut = (plan.scope_out as string[]) || [];
  const guardrails = (plan.guardrails as string[]) || [];
  const tasks = (plan.tasks as PlanTaskDisplay[]) || [];
  return { planTitle, estimatedEffort, overview, scopeIn, scopeOut, guardrails, tasks };
}

export function PlanReviewDialog({
  reviewId,
  plan,
  onApprove,
  onReject,
}: PlanReviewDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const [showFeedback, setShowFeedback] = useState(false);
  const [feedback, setFeedback] = useState('');
  const feedbackRef = useRef<HTMLTextAreaElement>(null);

  const { planTitle, estimatedEffort, overview, scopeIn, scopeOut, guardrails, tasks } =
    extractPlanFields(plan);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (showFeedback) return;
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onApprove(reviewId);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setShowFeedback(true);
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [reviewId, onApprove, showFeedback]);

  useEffect(() => {
    if (showFeedback) feedbackRef.current?.focus();
  }, [showFeedback]);

  useEffect(() => { dialogRef.current?.focus(); }, []);

  const handleReject = () => { onReject(reviewId, feedback); };

  return (
    <div className="plan-review-dialog" ref={dialogRef} tabIndex={-1}>
      <div className="plan-review-header">
        <ClipboardList size={16} className="plan-review-header-icon" />
        <span className="plan-review-header-title">Plan Review</span>
        {estimatedEffort && (
          <span className="plan-review-effort">{estimatedEffort}</span>
        )}
      </div>

      <div className="plan-review-body">
        <div className="plan-review-plan-title">{planTitle}</div>
        {overview && <div className="plan-review-overview">{overview}</div>}

        {scopeIn.length > 0 && (
          <div className="plan-review-scope">
            <span className="plan-review-scope-label">In scope:</span>
            <ul className="plan-review-scope-list">
              {scopeIn.map((item, i) => <li key={i}>{item}</li>)}
            </ul>
          </div>
        )}

        {scopeOut.length > 0 && (
          <div className="plan-review-scope">
            <span className="plan-review-scope-label">Out of scope:</span>
            <ul className="plan-review-scope-list">
              {scopeOut.map((item, i) => <li key={i}>{item}</li>)}
            </ul>
          </div>
        )}

        {guardrails.length > 0 && (
          <div className="plan-review-scope">
            <span className="plan-review-scope-label">Guardrails:</span>
            <ul className="plan-review-scope-list">
              {guardrails.map((item, i) => <li key={i}>{item}</li>)}
            </ul>
          </div>
        )}

        {tasks.length > 0 && (
          <div className="plan-review-tasks">
            {tasks.map((task) => (
              <PlanTaskItem key={task.id} task={task} />
            ))}
          </div>
        )}
      </div>

      {showFeedback && (
        <div className="plan-review-feedback">
          <textarea
            ref={feedbackRef}
            className="plan-review-feedback-input"
            placeholder="Why are you rejecting? (optional)"
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                handleReject();
              }
            }}
            rows={2}
          />
        </div>
      )}

      <div className="plan-review-footer">
        <span className="plan-review-hint">
          {showFeedback ? 'Enter = Reject' : 'Enter = Approve, Esc = Reject'}
        </span>
        <div className="plan-review-actions">
          {showFeedback ? (
            <button className="plan-review-btn plan-review-btn--reject" onClick={handleReject}>
              <X size={14} /> Reject
            </button>
          ) : (
            <>
              <button className="plan-review-btn plan-review-btn--reject" onClick={() => setShowFeedback(true)}>
                <X size={14} /> Reject
              </button>
              <button className="plan-review-btn plan-review-btn--approve" onClick={() => onApprove(reviewId)}>
                <Check size={14} /> Approve
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
