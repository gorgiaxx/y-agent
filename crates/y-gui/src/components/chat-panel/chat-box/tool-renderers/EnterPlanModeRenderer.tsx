import { useState } from 'react';
import { ScanSearch, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractEnterPlanModeMeta } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function EnterPlanModeRenderer({
  toolCall, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const meta = extractEnterPlanModeMeta(toolCall.arguments);
  const title = meta?.title ?? 'Plan Mode';
  const reason = meta?.reason ?? '';

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = !!reason || hasExpandable;

  return (
    <div className={`tool-call-plan-wrapper ${statusClass}`}>
      <div
        className="tool-call-plan-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={reason || 'EnterPlanMode'}
      >
        <span className="tool-call-plan-action-group">
          <ScanSearch size={14} className="tool-call-plan-icon" />
          <span className="tool-call-plan-action">Analyze</span>
        </span>
        <span className="tool-call-plan-title">{title}</span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
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
          {reason ? (
            <div className="tool-call-plan-reason">{reason}</div>
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
