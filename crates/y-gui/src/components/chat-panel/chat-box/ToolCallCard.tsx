import { useMemo } from 'react';
import { CheckCircle, XCircle, Loader } from 'lucide-react';
import type { ToolCallBrief } from '../../../types';
import { formatArguments, formatResult, formatResultFormatted } from './toolCallUtils';
import { TOOL_RENDERERS, DefaultRenderer } from './tool-renderers';
import './ToolCallCard.css';

interface ToolCallCardProps {
  toolCall: ToolCallBrief;
  status?: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
  /** Compact URL metadata JSON from the backend (survives truncation). */
  urlMeta?: string;
}

export function ToolCallCard({
  toolCall,
  status = 'success',
  result,
  durationMs,
  urlMeta,
}: ToolCallCardProps) {
  const statusIcon = {
    running: <Loader size={13} className="collapsible-card-spinner" />,
    success: <CheckCircle size={13} />,
    error: <XCircle size={13} />,
  }[status];

  const statusClass = `tool-status-${status}`;

  const displayArgs = useMemo(
    () => formatArguments(toolCall.name, toolCall.arguments),
    [toolCall.name, toolCall.arguments],
  );

  const displayResult = useMemo(
    () => (result ? formatResult(toolCall.name, result) : null),
    [toolCall.name, result],
  );

  const displayResultFormatted_ = useMemo(
    () => (result ? formatResultFormatted(toolCall.name, result, toolCall.arguments) : null),
    [toolCall.name, result, toolCall.arguments],
  );

  const Renderer = TOOL_RENDERERS[toolCall.name] ?? DefaultRenderer;

  return (
    <Renderer
      toolCall={toolCall}
      status={status}
      result={result}
      durationMs={durationMs}
      urlMeta={urlMeta}
      statusIcon={statusIcon}
      statusClass={statusClass}
      displayArgs={displayArgs}
      displayResult={displayResult}
      displayResultFormatted={displayResultFormatted_}
    />
  );
}
