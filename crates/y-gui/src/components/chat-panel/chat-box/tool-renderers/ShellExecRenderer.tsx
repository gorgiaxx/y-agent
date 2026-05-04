import { useState } from 'react';
import { SquareTerminal, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractShellCommand, extractShellExecSummary } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function ShellExecRenderer({
  toolCall, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const shellCommand = extractShellCommand(toolCall.arguments);
  const shellSummary = extractShellExecSummary(toolCall.arguments);
  const title = shellCommand
    ?? [shellSummary.label, shellSummary.processId].filter(Boolean).join(' ');
  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  return (
    <div className={`tool-call-shell-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={title}
      >
        <SquareTerminal size={14} className="tool-call-icon-muted" />
        {shellCommand ? (
          <span className="tool-call-monospace-value">{shellCommand}</span>
        ) : (
          <>
            <span className="tool-call-key">{shellSummary.label}</span>
            {shellSummary.processId && (
              <span className="tool-call-monospace-value">{shellSummary.processId}</span>
            )}
          </>
        )}
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        <span className={`tool-call-chevron ${expanded ? 'expanded' : ''}`}>
          <ChevronRight size={12} />
        </span>
      </div>
      {expanded && hasExpandable && (
        <div className="tool-call-detail">
          <DetailSections
            displayArgs={displayArgs}
            displayResult={activeResult}
            argsLabel="Command"
            resultLabel="Output"
            showRaw={showRaw}
            onToggleRaw={() => setShowRaw(!showRaw)}
          />
        </div>
      )}
    </div>
  );
}
