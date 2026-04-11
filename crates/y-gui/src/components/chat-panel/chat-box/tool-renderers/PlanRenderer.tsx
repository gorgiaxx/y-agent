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
  extractPlanDisplayMeta,
  extractPlanRequestMeta,
  type PlanTaskDisplay,
} from '../toolCallUtils';
import { DetailSections } from './shared';
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
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [showRaw, setShowRaw] = useState(false);

  const effectiveStatus = meta?.kind === 'plan_stage' && meta.stageStatus === 'running'
    ? 'running'
    : meta?.kind === 'plan_execution'
      && (
        meta.tasks.some((task) => task.status === 'in_progress')
        || (meta.completed + meta.failed) < (meta.totalPhases || meta.tasks.length)
      )
      ? 'running'
      : status;
  const effectiveStatusIcon = {
    running: <Loader2 size={13} className="collapsible-card-spinner" />,
    success: <CheckCircle2 size={13} />,
    error: <AlertCircle size={13} />,
  }[effectiveStatus];
  const effectiveStatusClass = `tool-status-${effectiveStatus}`;

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);

  let action = 'Plan';
  let title = requestMeta?.request ?? 'Plan';
  let path = '';
  let icon = <ClipboardList size={14} className="tool-call-plan-icon" />;
  let detail: ReactNode = null;

  if (meta?.kind === 'plan_stage' && meta.stage === 'plan_writer') {
    action = 'Plan';
    title = meta.planTitle || title;
    path = meta.planFile;
    detail = meta.planContent ? (
      <PlanMarkdownContent content={meta.planContent} />
    ) : null;
  } else if (meta?.kind === 'plan_stage' && meta.stage === 'task_decomposer') {
    action = 'Tasks';
    title = meta.planTitle || title;
    path = meta.planFile;
    icon = <ListTodo size={14} className="tool-call-plan-icon" />;
    detail = <PlanTaskList tasks={meta.tasks} />;
  } else if (meta?.kind === 'plan_execution') {
    action = 'Execute';
    title = meta.planTitle || title;
    path = meta.planFile;
    icon = <Play size={14} className="tool-call-plan-icon" />;
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

  const canExpand = detail != null || !!displayArgs || !!displayResult;

  return (
    <div className={`tool-call-plan-wrapper ${effectiveStatusClass}`}>
      <div
        className="tool-call-plan-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={path || title || 'Plan'}
      >
        <span className="tool-call-plan-action-group">
          {icon}
          <span className="tool-call-plan-action">{action}</span>
        </span>
        <span className="tool-call-plan-title">{title}</span>
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
          <span className={`tool-call-plan-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-plan-detail">
          {detail ?? fallbackDetail}
        </div>
      )}
    </div>
  );
}
