import { useState } from 'react';
import { Play, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractExitPlanModeMeta, basename } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function ExitPlanModeRenderer({
  toolCall, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const meta = extractExitPlanModeMeta(toolCall.arguments);
  const planFile = meta?.planFile ?? '';
  const totalPhases = meta?.totalPhases ?? 0;

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = hasExpandable;

  return (
    <div className={`tool-call-plan-wrapper ${statusClass}`}>
      <div
        className="tool-call-plan-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={planFile || 'ExitPlanMode'}
      >
        <span className="tool-call-plan-action-group">
          <Play size={14} className="tool-call-plan-icon" />
          <span className="tool-call-plan-action">Execute</span>
        </span>
        <span className="tool-call-plan-title">
          {totalPhases > 0 ? `${totalPhases} phase${totalPhases > 1 ? 's' : ''}` : 'Plan'}
        </span>
        {planFile && (
          <span className="tool-call-plan-path">{basename(planFile)}</span>
        )}
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
          <DetailSections
            displayArgs={displayArgs}
            displayResult={activeResult}
            showRaw={showRaw}
            onToggleRaw={() => setShowRaw(!showRaw)}
          />
        </div>
      )}
    </div>
  );
}
