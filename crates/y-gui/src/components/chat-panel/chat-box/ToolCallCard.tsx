import { useMemo } from 'react';
import { CheckCircle, XCircle, Loader } from 'lucide-react';
import type { ToolCallBrief } from '../../../types';
import {
  canonicalToolName,
  formatArguments,
  formatResult,
  formatResultFormatted,
} from './toolCallUtils';
import { TOOL_RENDERERS, DefaultRenderer } from './tool-renderers';
import './ToolCallCard.css';

interface ToolCallCardProps {
  toolCall: ToolCallBrief;
  status?: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
  /** Compact URL metadata JSON from the backend (survives truncation). */
  urlMeta?: string;
  /** Optional structured metadata for tool-specific renderers. */
  metadata?: Record<string, unknown>;
}

export function ToolCallCard({
  toolCall,
  status = 'success',
  result,
  durationMs,
  urlMeta,
  metadata,
}: ToolCallCardProps) {
  const canonicalName = useMemo(
    () => canonicalToolName(toolCall.name),
    [toolCall.name],
  );
  const normalizedToolCall = useMemo(
    () => ({ ...toolCall, name: canonicalName }),
    [canonicalName, toolCall],
  );

  const statusIcon = {
    running: <Loader size={13} className="tool-call-spinner" />,
    success: <CheckCircle size={13} />,
    error: <XCircle size={13} />,
  }[status];

  const statusClass = `tool-status-${status}`;

  const displayArgs = useMemo(
    () => formatArguments(canonicalName, toolCall.arguments),
    [canonicalName, toolCall.arguments],
  );

  const displayResult = useMemo(
    () => (result ? formatResult(canonicalName, result) : null),
    [canonicalName, result],
  );

  const displayResultFormatted_ = useMemo(
    () => (result ? formatResultFormatted(canonicalName, result, toolCall.arguments) : null),
    [canonicalName, result, toolCall.arguments],
  );

  const Renderer = TOOL_RENDERERS[canonicalName] ?? DefaultRenderer;

  return (
    <Renderer
      toolCall={normalizedToolCall}
      status={status}
      result={result}
      durationMs={durationMs}
      urlMeta={urlMeta}
      metadata={metadata}
      statusIcon={statusIcon}
      statusClass={statusClass}
      displayArgs={displayArgs}
      displayResult={displayResult}
      displayResultFormatted={displayResultFormatted_}
    />
  );
}
