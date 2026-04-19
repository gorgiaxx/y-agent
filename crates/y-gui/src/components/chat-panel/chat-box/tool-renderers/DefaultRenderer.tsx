import { useState } from 'react';
import { Wrench, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function DefaultRenderer({
  toolCall, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  return (
    <div className={`tool-call-default-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={toolCall.name}
      >
        <span className="tool-call-action-group">
          <Wrench size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">{toolCall.name}</span>
        </span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        {hasExpandable && (
          <span className={`tool-call-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && hasExpandable && (
        <div className="tool-call-detail">
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
