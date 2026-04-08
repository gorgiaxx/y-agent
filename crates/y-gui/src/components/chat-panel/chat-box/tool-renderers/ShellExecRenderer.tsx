import { useState } from 'react';
import { SquareTerminal, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractShellCommand } from '../toolCallUtils';
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
  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  return (
    <div className={`tool-call-shell-wrapper ${statusClass}`}>
      <div
        className="tool-call-shell-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={shellCommand ?? ''}
      >
        <SquareTerminal size={14} className="tool-call-shell-icon" />
        <span className="tool-call-shell-command">{shellCommand}</span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        <span className={`tool-call-shell-chevron ${expanded ? 'expanded' : ''}`}>
          <ChevronRight size={12} />
        </span>
      </div>
      {expanded && hasExpandable && (
        <div className="tool-call-shell-detail">
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
