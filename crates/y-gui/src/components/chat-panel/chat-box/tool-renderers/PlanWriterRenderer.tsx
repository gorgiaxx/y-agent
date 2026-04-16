import { useState, useMemo } from 'react';
import { ClipboardList, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractPlanWriterMeta, parsePlanWriterResult, basename } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function PlanWriterRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const meta = extractPlanWriterMeta(toolCall.arguments);
  const planResult = useMemo(
    () => (result ? parsePlanWriterResult(result) : null),
    [result],
  );

  const title = meta?.title ?? 'Plan';
  const planContent = meta?.content ?? '';
  const writtenPath = planResult?.path ?? '';

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = !!planContent || hasExpandable;

  return (
    <div className={`tool-call-plan-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={writtenPath || 'PlanWriter'}
      >
        <span className="tool-call-action-group">
          <ClipboardList size={14} className="tool-call-icon-accent" />
          <span className="tool-call-key">Plan</span>
        </span>
        <span className="tool-call-monospace-value">{title}</span>
        {writtenPath && (
          <span className="tool-call-plan-path">{basename(writtenPath)}</span>
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
          {planContent ? (
            <pre className="tool-call-plan-content">{planContent}</pre>
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
