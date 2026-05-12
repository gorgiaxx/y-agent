import { useRef, useState } from 'react';
import { Check, Edit3, X } from 'lucide-react';
import './PlanReviewInline.css';

interface PlanReviewInlineProps {
  reviewId: string;
  onApprove: (reviewId: string) => void;
  onRevise: (reviewId: string, feedback: string) => void;
  onReject: (reviewId: string, feedback: string) => void;
}

export function PlanReviewInline({
  reviewId,
  onApprove,
  onRevise,
  onReject,
}: PlanReviewInlineProps) {
  const [feedback, setFeedback] = useState('');
  const [showRejectInput, setShowRejectInput] = useState(false);
  const [rejectReason, setRejectReason] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const rejectInputRef = useRef<HTMLTextAreaElement>(null);

  const hasFeedback = feedback.trim().length > 0;

  const handleSubmit = () => {
    if (hasFeedback) {
      onRevise(reviewId, feedback.trim());
    } else {
      onApprove(reviewId);
    }
  };

  const handleReject = () => {
    if (!showRejectInput) {
      setShowRejectInput(true);
      requestAnimationFrame(() => rejectInputRef.current?.focus());
      return;
    }
    onReject(reviewId, rejectReason.trim());
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleRejectKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onReject(reviewId, rejectReason.trim());
    }
    if (e.key === 'Escape') {
      setShowRejectInput(false);
      setRejectReason('');
    }
  };

  return (
    <div className="plan-review-inline">
      <div className="plan-review-inline-input-row">
        <textarea
          ref={textareaRef}
          className="plan-review-inline-textarea"
          placeholder="Approve as-is, or describe changes..."
          value={feedback}
          onChange={(e) => setFeedback(e.target.value)}
          onKeyDown={handleKeyDown}
          rows={2}
        />
      </div>
      {showRejectInput && (
        <div className="plan-review-inline-reject-row">
          <textarea
            ref={rejectInputRef}
            className="plan-review-inline-textarea plan-review-inline-textarea--reject"
            placeholder="Reason for rejection (optional)"
            value={rejectReason}
            onChange={(e) => setRejectReason(e.target.value)}
            onKeyDown={handleRejectKeyDown}
            rows={1}
          />
        </div>
      )}
      <div className="plan-review-inline-actions">
        <span className="plan-review-inline-hint">
          {showRejectInput
            ? 'Enter = Reject, Esc = Cancel'
            : hasFeedback
              ? 'Enter = Revise'
              : 'Enter = Approve'}
        </span>
        <div className="plan-review-inline-buttons">
          <button
            type="button"
            className="plan-review-inline-btn plan-review-inline-btn--reject"
            onClick={handleReject}
          >
            <X size={13} />
            Reject
          </button>
          <button
            type="button"
            className={`plan-review-inline-btn ${
              hasFeedback
                ? 'plan-review-inline-btn--revise'
                : 'plan-review-inline-btn--approve'
            }`}
            onClick={handleSubmit}
          >
            {hasFeedback ? <Edit3 size={13} /> : <Check size={13} />}
            {hasFeedback ? 'Revise' : 'Approve'}
          </button>
        </div>
      </div>
    </div>
  );
}
