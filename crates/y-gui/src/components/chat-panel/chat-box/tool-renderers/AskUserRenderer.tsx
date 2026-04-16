import { useState, useMemo } from 'react';
import { MessageCircleQuestion, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractAskUserMeta, parseAskUserResult } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function AskUserRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const askUserMeta = extractAskUserMeta(toolCall.arguments, result);
  const askUserResult = useMemo(
    () => (result ? parseAskUserResult(result) : null),
    [result],
  );

  const questionCount = askUserMeta?.questions.length ?? 0;
  const isPending = askUserMeta?.status === 'pending';
  const hasAnswers = askUserResult && Object.keys(askUserResult.answers).length > 0;

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = !!askUserMeta || hasExpandable;

  return (
    <div className={`tool-call-askuser-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title="AskUser"
      >
        <span className="tool-call-action-group">
          <MessageCircleQuestion size={14} className="tool-call-icon-accent" />
          <span className="tool-call-key">Ask</span>
        </span>
        <span className="tool-call-monospace-value">
          {questionCount > 0
            ? `${questionCount} question${questionCount > 1 ? 's' : ''}`
            : 'AskUser'
          }
        </span>
        {isPending && (
          <span className="tool-call-askuser-pending">waiting</span>
        )}
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        {canExpand && (
          <span className={`tool-call-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-detail">
          {hasAnswers ? (
            <div className="tool-call-askuser-answers">
              {Object.entries(askUserResult!.answers).map(([q, a], i) => (
                <div key={i} className="tool-call-askuser-answer-row">
                  <div className="tool-call-askuser-answer-q">{q}</div>
                  <div className="tool-call-askuser-answer-a">{a || '(no answer)'}</div>
                </div>
              ))}
            </div>
          ) : askUserMeta ? (
            <div className="tool-call-askuser-questions">
              {askUserMeta.questions.map((q, qi) => (
                <div key={qi} className="tool-call-askuser-question-block">
                  <div className="tool-call-askuser-question-text">{q.question}</div>
                  <div className="tool-call-askuser-question-options">
                    {q.options.map((opt, oi) => (
                      <span key={oi} className="tool-call-askuser-option-chip">
                        {q.multi_select && (
                          <span className="tool-call-askuser-option-multi-marker" />
                        )}
                        {opt}
                      </span>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <DetailSections
              displayArgs={displayArgs}
              displayResult={activeResult}
              showRaw={showRaw}
              onToggleRaw={() => setShowRaw(!showRaw)}
            />
          )}
        </div>
      )}
    </div>
  );
}
