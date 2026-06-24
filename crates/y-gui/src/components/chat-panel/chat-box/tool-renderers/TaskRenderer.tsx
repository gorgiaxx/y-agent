import { useState } from 'react';
import { Bot, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { tryParseJson } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

function extractAgentName(argsRaw: string): string | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  if (typeof obj.agent_name === 'string' && obj.agent_name) return obj.agent_name;
  return null;
}

export function TaskRenderer({
  toolCall, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const agentName = extractAgentName(toolCall.arguments);
  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  return (
    <div className={`tool-call-default-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={agentName ? `Task: ${agentName}` : 'Task'}
      >
        <span className="tool-call-action-group">
          <Bot size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">Task</span>
        </span>
        {agentName && (
          <span className="tool-call-monospace-value">{agentName}</span>
        )}
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