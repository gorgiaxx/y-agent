import { useMemo, useState, type ReactNode } from 'react';
import {
  AlertCircle,
  CheckCircle2,
  ChevronRight,
  Circle,
  Loader2,
  RefreshCw,
  Search,
} from 'lucide-react';

import { formatDuration } from '../../../../utils/formatDuration';
import {
  extractLoopDisplayMeta,
  extractLoopRequestMeta,
  type LoopRoundDisplay,
} from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';
import './LoopRenderer.css';

function RoundStatusIcon({ status }: { status: string }) {
  if (status === 'completed') {
    return <CheckCircle2 size={14} className="tool-call-loop-round-icon" />;
  }
  if (status === 'failed' || status === 'cancelled') {
    return <AlertCircle size={14} className="tool-call-loop-round-icon" />;
  }
  return <Loader2 size={14} className="tool-call-loop-round-icon tool-call-loop-round-icon--spinning" />;
}

function RoundTimeline({ rounds, maxRounds }: { rounds: LoopRoundDisplay[]; maxRounds: number }) {
  if (rounds.length === 0) {
    return <div className="tool-call-loop-empty">No rounds completed yet.</div>;
  }

  return (
    <div className="tool-call-loop-timeline">
      {rounds.map((round) => {
        const done = round.tasksCompleted.length;
        const remaining = round.tasksRemaining.length;
        const total = done + remaining;

        return (
          <div
            key={round.round}
            className={`tool-call-loop-round tool-call-loop-round--${round.status}`}
          >
            <div className="tool-call-loop-round-header">
              <RoundStatusIcon status={round.status} />
              <span className="tool-call-loop-round-label">
                Round {round.round}/{maxRounds}
              </span>
              {total > 0 && (
                <span className="tool-call-loop-round-progress">
                  {done}/{total} tasks
                </span>
              )}
              {round.durationMs > 0 && (
                <span className="tool-call-loop-round-duration">
                  {formatDuration(round.durationMs)}
                </span>
              )}
            </div>
            {(round.tasksCompleted.length > 0 || round.tasksRemaining.length > 0) && (
              <div className="tool-call-loop-round-tasks">
                {round.tasksCompleted.map((task, i) => (
                  <div key={`done-${i}`} className="tool-call-loop-task tool-call-loop-task--done">
                    <CheckCircle2 size={11} />
                    <span>{task}</span>
                  </div>
                ))}
                {round.tasksRemaining.map((task, i) => (
                  <div key={`todo-${i}`} className="tool-call-loop-task tool-call-loop-task--todo">
                    <Circle size={11} />
                    <span>{task}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function LoopRenderer({
  toolCall,
  status,
  durationMs,
  result,
  metadata,
  displayArgs,
  displayResult,
  displayResultFormatted,
}: ToolRendererProps) {
  const meta = useMemo(
    () => extractLoopDisplayMeta(metadata, result),
    [metadata, result],
  );
  const requestMeta = useMemo(
    () => extractLoopRequestMeta(toolCall.arguments),
    [toolCall.arguments],
  );

  const defaultExpanded = meta?.kind === 'loop_round' || meta?.kind === 'loop_execution';
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [showRaw, setShowRaw] = useState(false);

  const effectiveStatus = meta?.kind === 'loop_round' && meta.roundStatus === 'running'
    ? 'running'
    : meta?.kind === 'loop_review' && meta.reviewStatus === 'running'
      ? 'running'
      : meta?.kind === 'loop_init'
        ? 'running'
        : status;
  const effectiveStatusIcon = {
    running: <Loader2 size={13} className="tool-call-spinner" />,
    success: <CheckCircle2 size={13} />,
    error: <AlertCircle size={13} />,
  }[effectiveStatus];
  const effectiveStatusClass = `tool-status-${effectiveStatus}`;

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);

  let action = 'Loop';
  let title = requestMeta?.request ?? 'Loop';
  let icon: ReactNode = <RefreshCw size={14} className="tool-call-icon-accent" />;
  let detail: ReactNode = null;

  if (meta?.kind === 'loop_init') {
    action = 'Loop';
    title = meta.request || title;
    detail = (
      <div className="tool-call-loop-summary">
        <div className="tool-call-loop-stats">
          <span className="tool-call-loop-stat">Max {meta.maxRounds} rounds</span>
        </div>
      </div>
    );
  } else if (meta?.kind === 'loop_round') {
    action = `Round ${meta.round}`;
    detail = (
      <div className="tool-call-loop-summary">
        <div className="tool-call-loop-stats">
          <span className="tool-call-loop-stat">
            {meta.tasksCompleted.length} completed
          </span>
          {meta.tasksRemaining.length > 0 && (
            <span className="tool-call-loop-stat">
              {meta.tasksRemaining.length} remaining
            </span>
          )}
          {meta.converged && (
            <span className="tool-call-loop-stat tool-call-loop-stat--converged">
              Converged
            </span>
          )}
        </div>
        <RoundTimeline rounds={meta.rounds} maxRounds={meta.maxRounds} />
      </div>
    );
  } else if (meta?.kind === 'loop_review') {
    action = 'Review';
    icon = <Search size={14} className="tool-call-icon-accent" />;
    detail = (
      <div className="tool-call-loop-summary">
        <div className="tool-call-loop-stats">
          <span className="tool-call-loop-stat">
            {meta.totalRounds} rounds completed
          </span>
          <span className={`tool-call-loop-stat tool-call-loop-stat--${meta.reviewStatus}`}>
            Review {meta.reviewStatus}
          </span>
        </div>
      </div>
    );
  } else if (meta?.kind === 'loop_execution') {
    action = meta.finalStatus === 'converged' ? 'Converged' : 'Budget';
    detail = (
      <div className="tool-call-loop-summary">
        <div className="tool-call-loop-stats">
          <span className="tool-call-loop-stat">
            {meta.totalRounds}/{meta.maxRounds} rounds
          </span>
          <span className={`tool-call-loop-stat tool-call-loop-stat--${meta.finalStatus}`}>
            {meta.finalStatus === 'converged' ? 'Converged' : meta.finalStatus === 'budget_exhausted' ? 'Budget exhausted' : meta.finalStatus}
          </span>
        </div>
        <RoundTimeline rounds={meta.rounds} maxRounds={meta.maxRounds} />
      </div>
    );
  }

  const fallbackDetail = (
    <DetailSections
      displayArgs={displayArgs}
      displayResult={activeResult}
      showRaw={showRaw}
      onToggleRaw={() => setShowRaw(!showRaw)}
    />
  );

  const canExpand = detail != null || !!displayArgs || !!displayResult;

  return (
    <div className={`tool-call-loop-wrapper ${effectiveStatusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={title || 'Loop'}
      >
        <span className="tool-call-action-group">
          {icon}
          <span className="tool-call-key">{action}</span>
        </span>
        <span className="tool-call-monospace-value">{title}</span>
        <span className={`tool-call-status-icon ${effectiveStatusClass}`}>{effectiveStatusIcon}</span>
        <span className={`tool-call-status ${effectiveStatusClass}`}>
          {{ running: 'Running...', success: 'Done', error: 'Failed' }[effectiveStatus]}
        </span>
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
          {detail ?? fallbackDetail}
        </div>
      )}
    </div>
  );
}
