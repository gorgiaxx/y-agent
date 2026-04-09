import { useMemo, useState, type ReactNode } from 'react';
import {
  ClipboardList,
  ChevronRight,
  ListTodo,
  Play,
} from 'lucide-react';

import { formatDuration } from '../../../../utils/formatDuration';
import {
  basename,
  extractPlanDisplayMeta,
  extractPlanRequestMeta,
  type PlanTaskDisplay,
} from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

function PlanTaskList({ tasks }: { tasks: PlanTaskDisplay[] }) {
  if (tasks.length === 0) {
    return <div className="tool-call-plan-empty">No tasks were extracted.</div>;
  }

  return (
    <div className="tool-call-plan-task-list">
      {tasks.map((task) => (
        <div key={task.id || `${task.phase}-${task.title}`} className="tool-call-plan-task">
          <div className="tool-call-plan-task-head">
            <span className="tool-call-plan-task-phase">Phase {task.phase || '?'}</span>
            <span className="tool-call-plan-task-title">{task.title}</span>
          </div>
          {task.description && (
            <div className="tool-call-plan-task-desc">{task.description}</div>
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
  statusIcon,
  statusClass,
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

  const defaultExpanded = meta?.kind === 'plan_stage' && meta.stage === 'task_decomposer';
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [showRaw, setShowRaw] = useState(false);

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
      <pre className="tool-call-plan-content">{meta.planContent}</pre>
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
    <div className={`tool-call-plan-wrapper ${statusClass}`}>
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
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        <span className={`tool-call-status ${statusClass}`}>
          {{ running: 'Running...', success: 'Done', error: 'Failed' }[status]}
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
