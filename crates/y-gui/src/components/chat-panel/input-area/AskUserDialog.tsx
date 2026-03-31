import { useState, useCallback, useEffect, useRef } from 'react';
import { MessageCircleQuestion, ChevronLeft, ChevronRight } from 'lucide-react';
import './AskUserDialog.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AskUserQuestion {
  question: string;
  options: string[];
  multi_select?: boolean;
}

interface AskUserDialogProps {
  /** Unique interaction ID from the backend. */
  interactionId: string;
  /** Structured questions from the AskUser tool call. */
  questions: AskUserQuestion[];
  /** Callback when user submits answers. */
  onSubmit: (interactionId: string, answers: Record<string, string>) => void;
  /** Callback when user dismisses without answering. */
  onDismiss: (interactionId: string) => void;
}

// Other option sentinel
const OTHER_LABEL = '__other__';

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * AskUserDialog -- floating panel above InputArea for LLM-initiated questions.
 *
 * Renders one question at a time with step navigation (Back / Next).
 * Each question supports single-select (radio) or multi-select (checkbox)
 * with an "Other" free-text option always appended.
 *
 * Single-select auto-advances to the next question after selection.
 * On the last question, single-select auto-submits.
 *
 * Keyboard navigation: Arrow keys move focus, Space/Enter select, Escape dismisses.
 */
export function AskUserDialog({ interactionId, questions, onSubmit, onDismiss }: AskUserDialogProps) {
  // Per-question state: selected option labels (array for multi-select support).
  const [selections, setSelections] = useState<string[][]>(() =>
    questions.map(() => [])
  );
  // Per-question "Other" text.
  const [otherTexts, setOtherTexts] = useState<string[]>(() =>
    questions.map(() => '')
  );
  // Current question step (0-indexed).
  const [currentStep, setCurrentStep] = useState(0);
  // Slide direction for transition animation.
  const [slideDir, setSlideDir] = useState<'forward' | 'back'>('forward');
  // Currently focused option index for keyboard navigation.
  const [focusedO, setFocusedO] = useState(0);

  const dialogRef = useRef<HTMLDivElement>(null);
  const otherInputRefs = useRef<(HTMLInputElement | null)[]>([]);

  const currentQ = questions[currentStep];
  const isMulti = currentQ?.multi_select ?? false;
  const isLastStep = currentStep === questions.length - 1;

  // Total option count for current question (including "Other").
  const optionCount = currentQ ? currentQ.options.length + 1 : 0;

  // Toggle selection for an option.
  const toggleSelection = useCallback((qi: number, label: string) => {
    setSelections(prev => {
      const next = [...prev];
      const q = questions[qi];
      if (q.multi_select) {
        // Multi-select: toggle in/out.
        if (next[qi].includes(label)) {
          next[qi] = next[qi].filter(l => l !== label);
        } else {
          if (label !== OTHER_LABEL) {
            next[qi] = [...next[qi].filter(l => l !== OTHER_LABEL), label];
          } else {
            next[qi] = [...next[qi], label];
          }
        }
      } else {
        // Single-select: replace.
        next[qi] = [label];
      }
      return next;
    });
  }, [questions]);

  // Update "Other" text.
  const setOtherText = useCallback((qi: number, text: string) => {
    setOtherTexts(prev => {
      const next = [...prev];
      next[qi] = text;
      return next;
    });
  }, []);

  // Navigate to next step.
  const goNext = useCallback(() => {
    if (currentStep < questions.length - 1) {
      setSlideDir('forward');
      setCurrentStep(prev => prev + 1);
      setFocusedO(0);
    }
  }, [currentStep, questions.length]);

  // Navigate to previous step.
  const goBack = useCallback(() => {
    if (currentStep > 0) {
      setSlideDir('back');
      setCurrentStep(prev => prev - 1);
      setFocusedO(0);
    }
  }, [currentStep]);

  // Build answers and submit.
  const handleConfirm = useCallback(() => {
    const answers: Record<string, string> = {};

    for (let qi = 0; qi < questions.length; qi++) {
      const q = questions[qi];
      const sel = selections[qi];
      const resolvedLabels = sel.map(l =>
        l === OTHER_LABEL ? otherTexts[qi] || '(no custom input)' : l
      );
      answers[q.question] = resolvedLabels.join(', ');
    }

    onSubmit(interactionId, answers);
  }, [questions, selections, otherTexts, interactionId, onSubmit]);

  // Handle option click with auto-advance / auto-submit for single-select.
  const handleOptionClick = useCallback((qi: number, label: string) => {
    toggleSelection(qi, label);

    // Single-select non-other: auto-advance or auto-submit.
    if (!questions[qi].multi_select && label !== OTHER_LABEL) {
      setTimeout(() => {
        if (qi < questions.length - 1) {
          // Auto-advance to next question.
          setSlideDir('forward');
          setCurrentStep(qi + 1);
          setFocusedO(0);
        } else {
          // Last question -- auto-submit.
          const answers: Record<string, string> = {};
          for (let i = 0; i < questions.length; i++) {
            const q = questions[i];
            // Use the just-clicked label for the current question.
            const sel = i === qi ? [label] : selections[i];
            const resolvedLabels = sel.map(l =>
              l === OTHER_LABEL ? otherTexts[i] || '(no custom input)' : l
            );
            answers[q.question] = resolvedLabels.join(', ');
          }
          onSubmit(interactionId, answers);
        }
      }, 180);
    }
  }, [questions, selections, otherTexts, interactionId, onSubmit, toggleSelection]);

  // Check if current step is answered (for enabling Next/Confirm).
  const isCurrentStepAnswered = (() => {
    const sel = selections[currentStep];
    if (!sel || sel.length === 0) return false;
    if (sel.includes(OTHER_LABEL) && !otherTexts[currentStep].trim()) return false;
    return true;
  })();

  // Check if all questions are answered (for confirm button on last step).
  const canConfirm = selections.every((sel, qi) => {
    if (sel.length === 0) return false;
    if (sel.includes(OTHER_LABEL) && !otherTexts[qi].trim()) return false;
    return true;
  });

  // Keyboard navigation.
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case 'ArrowUp':
          e.preventDefault();
          setFocusedO(prev => Math.max(0, prev - 1));
          break;
        case 'ArrowDown':
          e.preventDefault();
          setFocusedO(prev => Math.min(optionCount - 1, prev + 1));
          break;
        case 'ArrowLeft':
          e.preventDefault();
          goBack();
          break;
        case 'ArrowRight':
          e.preventDefault();
          if (isCurrentStepAnswered) goNext();
          break;
        case ' ':
        case 'Enter': {
          e.preventDefault();
          if (!currentQ) break;
          const isOther = focusedO === currentQ.options.length;
          const label = isOther ? OTHER_LABEL : currentQ.options[focusedO];
          handleOptionClick(currentStep, label);
          // If Other selected, focus the text input.
          if (isOther) {
            setTimeout(() => {
              otherInputRefs.current[currentStep]?.focus();
            }, 0);
          }
          break;
        }
        case 'Escape':
          e.preventDefault();
          onDismiss(interactionId);
          break;
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [currentStep, focusedO, optionCount, currentQ, isCurrentStepAnswered, handleOptionClick, goBack, goNext, onDismiss, interactionId]);

  // Auto-focus dialog on mount.
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  return (
    <div className="ask-user-dialog" ref={dialogRef} tabIndex={-1}>
      {/* Header */}
      <div className="ask-user-header">
        <MessageCircleQuestion size={16} className="ask-user-header-icon" />
        <span className="ask-user-header-title">Question from Assistant</span>
        {questions.length > 1 && (
          <span className="ask-user-header-step">
            {currentStep + 1} / {questions.length}
          </span>
        )}
      </div>

      {/* Progress bar (multi-question only) */}
      {questions.length > 1 && (
        <div className="ask-user-progress-track">
          <div
            className="ask-user-progress-fill"
            style={{ width: `${((currentStep + 1) / questions.length) * 100}%` }}
          />
        </div>
      )}

      {/* Body -- single question card */}
      <div className="ask-user-body">
        <div
          key={currentStep}
          className={`ask-user-card ask-user-card--${slideDir}`}
        >
          <QuestionCard
            question={currentQ}
            qi={currentStep}
            selections={selections[currentStep]}
            otherText={otherTexts[currentStep]}
            focusedO={focusedO}
            onToggle={handleOptionClick}
            onOtherText={setOtherText}
            onFocus={(oIdx) => setFocusedO(oIdx)}
            otherInputRef={(el) => { otherInputRefs.current[currentStep] = el; }}
          />
        </div>
      </div>

      {/* Footer */}
      <div className="ask-user-footer">
        <div className="ask-user-footer-left">
          {questions.length > 1 && currentStep > 0 ? (
            <button
              className="ask-user-btn ask-user-btn--nav"
              onClick={goBack}
            >
              <ChevronLeft size={14} />
              Back
            </button>
          ) : (
            <span className="ask-user-hint">
              {isMulti ? 'Select multiple, then confirm' : 'Click to select'}
            </span>
          )}
        </div>
        <div className="ask-user-footer-actions">
          <button
            className="ask-user-btn ask-user-btn--skip"
            onClick={() => onDismiss(interactionId)}
          >
            Skip
          </button>
          {isMulti && !isLastStep && (
            <button
              className="ask-user-btn ask-user-btn--confirm"
              onClick={goNext}
              disabled={!isCurrentStepAnswered}
            >
              Next
              <ChevronRight size={14} />
            </button>
          )}
          {isMulti && isLastStep && (
            <button
              className="ask-user-btn ask-user-btn--confirm"
              onClick={handleConfirm}
              disabled={!canConfirm}
            >
              Confirm
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// QuestionCard sub-component
// ---------------------------------------------------------------------------

interface QuestionCardProps {
  question: AskUserQuestion;
  qi: number;
  selections: string[];
  otherText: string;
  focusedO: number;
  onToggle: (qi: number, label: string) => void;
  onOtherText: (qi: number, text: string) => void;
  onFocus: (optionIndex: number) => void;
  otherInputRef: (el: HTMLInputElement | null) => void;
}

function QuestionCard({
  question,
  qi,
  selections,
  otherText,
  focusedO,
  onToggle,
  onOtherText,
  onFocus,
  otherInputRef,
}: QuestionCardProps) {
  const isMulti = question.multi_select ?? false;

  return (
    <div className="ask-user-question-section">
      <div className="ask-user-question-text">{question.question}</div>
      <div className="ask-user-options">
        {question.options.map((opt, oi) => {
          const isSelected = selections.includes(opt);
          const isFocused = focusedO === oi;
          return (
            <div
              key={oi}
              className={[
                'ask-user-option',
                isMulti ? 'ask-user-option--multi' : '',
                isSelected ? 'selected' : '',
                isFocused ? 'focused' : '',
              ].filter(Boolean).join(' ')}
              onClick={() => onToggle(qi, opt)}
              onMouseEnter={() => onFocus(oi)}
            >
              <div className="ask-user-option-indicator">
                <div className="ask-user-option-indicator-dot" />
              </div>
              <div className="ask-user-option-content">
                <span className="ask-user-option-label">{opt}</span>
              </div>
            </div>
          );
        })}

        {/* "Other" option */}
        {(() => {
          const otherIdx = question.options.length;
          const isOtherSelected = selections.includes(OTHER_LABEL);
          const isOtherFocused = focusedO === otherIdx;
          return (
            <div
              className={[
                'ask-user-option',
                isMulti ? 'ask-user-option--multi' : '',
                isOtherSelected ? 'selected' : '',
                isOtherFocused ? 'focused' : '',
              ].filter(Boolean).join(' ')}
              onClick={() => onToggle(qi, OTHER_LABEL)}
              onMouseEnter={() => onFocus(otherIdx)}
            >
              <div className="ask-user-option-indicator">
                <div className="ask-user-option-indicator-dot" />
              </div>
              <div className="ask-user-option-content">
                <span className="ask-user-option-label">Other</span>
                {isOtherSelected && (
                  <input
                    ref={otherInputRef}
                    className="ask-user-other-input"
                    type="text"
                    placeholder="Type your answer..."
                    value={otherText}
                    onChange={(e) => onOtherText(qi, e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      // Prevent Enter from toggling the option.
                      if (e.key === 'Enter') {
                        e.stopPropagation();
                      }
                      // Prevent arrow keys from navigating options while typing.
                      if (['ArrowUp', 'ArrowDown'].includes(e.key)) {
                        e.stopPropagation();
                      }
                    }}
                    autoFocus
                  />
                )}
              </div>
            </div>
          );
        })()}
      </div>
    </div>
  );
}
