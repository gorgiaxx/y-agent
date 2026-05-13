import { Fragment, useMemo, useState, type ReactNode } from 'react';
import {
  AlertCircle,
  CheckCircle2,
  ClipboardList,
  ChevronRight,
  Circle,
  ListTodo,
  Loader2,
  Play,
} from 'lucide-react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';

import { useResolvedTheme } from '../../../../hooks/useTheme';
import { formatDuration } from '../../../../utils/formatDuration';
import { MarkdownSegment } from '../MessageShared';
import { makeMarkdownComponents } from '../messageUtils';
import {
  basename,
} from '../toolCallUtils';
import {
  extractPlanDisplayMeta,
  extractPlanRequestMeta,
  type PlanWriterStageDisplay,
  type PlanTaskDisplay,
} from '../planToolDisplay';
import { DetailSections } from './shared';
import { PlanReviewInline } from './PlanReviewInline';
import { usePlanReview } from '../../planReviewState';
import type { ToolRendererProps } from './types';

function formatPlanTaskStatus(status: string): string {
  if (status === 'completed') return 'Completed';
  if (status === 'failed') return 'Failed';
  if (status === 'in_progress') return 'In Progress';
  return 'Pending';
}

function PlanTaskStatusIcon({ status }: { status: string }) {
  if (status === 'completed') {
    return <CheckCircle2 size={14} className="tool-call-plan-task-status-icon" />;
  }
  if (status === 'failed') {
    return <AlertCircle size={14} className="tool-call-plan-task-status-icon" />;
  }
  if (status === 'in_progress') {
    return <Loader2 size={14} className="tool-call-plan-task-status-icon tool-call-plan-task-status-icon--spinning" />;
  }
  return <Circle size={14} className="tool-call-plan-task-status-icon" />;
}

function PlanMarkdownContent({ content }: { content: string }) {
  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const markdownComponents = useMemo(
    () => makeMarkdownComponents(codeThemeStyle),
    [codeThemeStyle],
  );

  return (
    <div className="tool-call-plan-markdown markdown-body">
      <MarkdownSegment text={content} components={markdownComponents} />
    </div>
  );
}

function renderMultilineText(content: string): ReactNode {
  const lines = content.replace(/\r\n/g, '\n').split('\n');

  return lines.map((line, index) => (
    <Fragment key={`${index}-${line}`}>
      {index > 0 && <br />}
      {line}
    </Fragment>
  ));
}

export function PlanTaskItem({
  task,
  defaultExpanded = false,
}: {
  task: PlanTaskDisplay;
  defaultExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const statusLabel = formatPlanTaskStatus(task.status);
  const hasDetail = !!task.description || task.keyFiles.length > 0 || task.acceptanceCriteria.length > 0;

  const headerContent = (
    <>
      <span
        className={`tool-call-plan-task-status tool-call-plan-task-status-column tool-call-plan-task-status--${task.status}`}
        title={statusLabel}
        aria-label={statusLabel}
      >
        <PlanTaskStatusIcon status={task.status} />
        <span className="tool-call-plan-task-status-text">{statusLabel}</span>
      </span>
      <span className="tool-call-plan-task-main">
        <span className="tool-call-plan-task-phase">Phase {task.phase || '?'}</span>
        <span className="tool-call-plan-task-title">{task.title}</span>
      </span>
      {hasDetail && (
        <span className={`tool-call-plan-task-chevron ${expanded ? 'expanded' : ''}`}>
          <ChevronRight size={12} />
        </span>
      )}
    </>
  );

  return (
    <div className={`tool-call-plan-task ${expanded ? 'expanded' : ''}`}>
      {hasDetail ? (
        <button
          type="button"
          className="tool-call-plan-task-toggle"
          onClick={() => setExpanded(!expanded)}
          aria-expanded={expanded}
        >
          {headerContent}
        </button>
      ) : (
        <div className="tool-call-plan-task-static">{headerContent}</div>
      )}
      {expanded && hasDetail && (
        <div className="tool-call-plan-task-detail">
          {task.description && (
            <div className="tool-call-plan-task-desc">
              {renderMultilineText(task.description)}
            </div>
          )}
          {task.keyFiles.length > 0 && (
            <div className="tool-call-plan-task-meta">
              <span className="tool-call-plan-task-meta-label">Files</span>
              <span>{task.keyFiles.join(', ')}</span>
            </div>
          )}
          {task.acceptanceCriteria.length > 0 && (
            <div className="tool-call-plan-task-meta">
              <span className="tool-call-plan-task-meta-label">Criteria</span>
              <span>{task.acceptanceCriteria.join(' | ')}</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function PlanTaskList({ tasks }: { tasks: PlanTaskDisplay[] }) {
  if (tasks.length === 0) {
    return <div className="tool-call-plan-empty">No tasks were extracted.</div>;
  }

  return (
    <div className="tool-call-plan-task-list">
      {tasks.map((task) => (
        <PlanTaskItem
          key={task.id || `${task.phase}-${task.title}`}
          task={task}
        />
      ))}
    </div>
  );
}

function formatReviewStatus(status: string): string {
  if (status === 'approved') return 'Approved';
  if (status === 'auto_approved') return 'Auto approved';
  if (status === 'awaiting_user') return 'Awaiting review';
  if (status === 'feedback_received') return 'Feedback received';
  if (status === 'declined' || status === 'rejected') return 'Rejected';
  if (status === 'review_timeout') return 'Timed out';
  if (status === 'review_cancelled') return 'Cancelled';
  return '';
}

function PlanTextList({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null;

  return (
    <div className="tool-call-plan-text-section">
      <div className="tool-call-plan-section-label">{label}</div>
      <ul className="tool-call-plan-text-list">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </div>
  );
}

function PlanReviewPanel({
  meta,
}: {
  meta: PlanWriterStageDisplay;
}) {
  const planReview = usePlanReview();
  const reviewLabel = formatReviewStatus(meta.reviewStatus);
  const hasSummary = !!meta.overview
    || !!meta.estimatedEffort
    || meta.scopeIn.length > 0
    || meta.scopeOut.length > 0
    || meta.guardrails.length > 0
    || !!reviewLabel
    || !!meta.reviewFeedback;

  const isAwaitingReview =
    meta.reviewStatus === 'awaiting_user' && planReview?.reviewId;

  return (
    <div className="tool-call-plan-review">
      {hasSummary && (
        <div className="tool-call-plan-review-summary">
          {(reviewLabel || meta.estimatedEffort || meta.tasks.length > 0) && (
            <div className="tool-call-plan-review-meta">
              {reviewLabel && (
                <span className={`tool-call-plan-review-badge tool-call-plan-review-badge--${meta.reviewStatus}`}>
                  {reviewLabel}
                </span>
              )}
              {meta.estimatedEffort && (
                <span className="tool-call-plan-stat">{meta.estimatedEffort}</span>
              )}
              {meta.tasks.length > 0 && (
                <span className="tool-call-plan-stat">
                  {meta.tasks.length} {meta.tasks.length === 1 ? 'phase' : 'phases'}
                </span>
              )}
            </div>
          )}
          {meta.overview && (
            <p className="tool-call-plan-overview">{meta.overview}</p>
          )}
          {meta.reviewFeedback && (
            <div className="tool-call-plan-review-feedback">
              <span className="tool-call-plan-section-label">User feedback</span>
              <span>{meta.reviewFeedback}</span>
            </div>
          )}
          <PlanTextList label="Scope in" items={meta.scopeIn} />
          <PlanTextList label="Scope out" items={meta.scopeOut} />
          <PlanTextList label="Guardrails" items={meta.guardrails} />
        </div>
      )}
      <PlanTaskList tasks={meta.tasks} />
      {isAwaitingReview && (
        <PlanReviewInline
          reviewId={planReview.reviewId!}
          onApprove={planReview.onApprove}
          onRevise={planReview.onRevise}
          onReject={planReview.onReject}
        />
      )}
    </div>
  );
}

export function PlanRenderer({
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
    () => extractPlanDisplayMeta(metadata, result),
    [metadata, result],
  );
  const requestMeta = useMemo(
    () => extractPlanRequestMeta(toolCall.arguments),
    [toolCall.arguments],
  );

  const defaultExpanded = meta?.kind === 'plan_stage' || meta?.kind === 'plan_execution';
  const [showRaw, setShowRaw] = useState(false);
  const autoExpandKey = meta
    ? `${meta.kind}:${meta.planFile}`
    : null;
  const [expansionState, setExpansionState] = useState(() => ({
    key: autoExpandKey,
    expanded: defaultExpanded,
  }));
  const expanded = expansionState.key === autoExpandKey
    ? expansionState.expanded
    : defaultExpanded;
  const toggleExpanded = () => {
    setExpansionState({
      key: autoExpandKey,
      expanded: !expanded,
    });
  };

  const effectiveStatus = status === 'error'
    ? 'error'
    : meta?.kind === 'plan_stage' && meta.stageStatus === 'running'
      ? 'running'
      : meta?.kind === 'plan_execution'
        && (
          meta.tasks.some((task) => task.status === 'in_progress')
          || (meta.completed + meta.failed) < (meta.totalPhases || meta.tasks.length)
        )
        ? 'running'
        : status;
  const effectiveStatusIcon = {
    running: <Loader2 size={13} className="tool-call-spinner" />,
    success: <CheckCircle2 size={13} />,
    error: <AlertCircle size={13} />,
  }[effectiveStatus];
  const effectiveStatusClass = `tool-status-${effectiveStatus}`;

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);

  let action = 'Plan';
  let title = requestMeta?.request ?? 'Plan';
  let path = '';
  let icon = <ClipboardList size={14} className="tool-call-icon-accent" />;
  let detail: ReactNode = null;

  if (meta?.kind === 'plan_stage' && meta.stage === 'plan_writer') {
    action = meta.tasks.length > 0 ? 'Tasks' : 'Plan';
    title = meta.planTitle || title;
    path = meta.planFile;
    if (meta.tasks.length > 0) {
      icon = <ListTodo size={14} className="tool-call-icon-accent" />;
      detail = <PlanReviewPanel meta={meta} />;
    } else if (meta.planContent) {
      detail = <PlanMarkdownContent content={meta.planContent} />;
    }
  } else if (meta?.kind === 'plan_execution') {
    action = 'Execute';
    title = meta.planTitle || title;
    path = meta.planFile;
    icon = <Play size={14} className="tool-call-icon-accent" />;
    detail = (
      <div className="tool-call-plan-summary">
        <div className="tool-call-plan-stats">
          <span className="tool-call-plan-stat">
            {meta.completed}/{meta.totalPhases || meta.tasks.length} completed
          </span>
          {meta.failed > 0 && (
            <span className="tool-call-plan-stat tool-call-plan-stat--error">
              {meta.failed} failed
            </span>
          )}
        </div>
        <PlanTaskList tasks={meta.tasks} />
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
  const errorNotice = status === 'error' && result ? (
    <div className="tool-call-plan-error">
      <AlertCircle size={14} />
      <span>{result}</span>
    </div>
  ) : null;
  const detailContent = detail != null ? (
    <>
      {errorNotice}
      {detail}
    </>
  ) : (
    errorNotice ?? fallbackDetail
  );

  const canExpand = detail != null || errorNotice != null || !!displayArgs || !!displayResult;

  return (
    <div className={`tool-call-plan-wrapper ${effectiveStatusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && toggleExpanded()}
        title={path || title || 'Plan'}
      >
        <span className="tool-call-action-group">
          {icon}
          <span className="tool-call-key">{action}</span>
        </span>
        <span className="tool-call-monospace-value">{title}</span>
        {path && (
          <span className="tool-call-plan-path">{basename(path)}</span>
        )}
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
          {detailContent}
        </div>
      )}
    </div>
  );
}
