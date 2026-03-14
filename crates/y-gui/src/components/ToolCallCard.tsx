import { useState, useMemo } from 'react';
import { Wrench, ChevronRight, CheckCircle, XCircle, Loader } from 'lucide-react';
import type { ToolCallBrief } from '../types';
import './ToolCallCard.css';

interface ToolCallCardProps {
  toolCall: ToolCallBrief;
  status?: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
}

// ---------------------------------------------------------------------------
// Smart formatting helpers
// ---------------------------------------------------------------------------

/** Try to parse JSON; return null on failure. */
function tryParseJson(raw: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === 'object' && parsed !== null ? parsed : null;
  } catch {
    return null;
  }
}

/** Format arguments for display based on tool name. */
function formatArguments(name: string, raw: string): string {
  if (!raw) return '';
  const obj = tryParseJson(raw);
  if (!obj) return raw;

  // shell_exec: show only the command
  if (name === 'shell_exec' && typeof obj.command === 'string') {
    return obj.command;
  }

  // Default: pretty-print JSON
  return JSON.stringify(obj, null, 2);
}

interface FormattedResult {
  parts: Array<{ text: string; isStderr: boolean }>;
}

/** Format result for display based on tool name. */
function formatResult(name: string, raw: string): FormattedResult | null {
  if (!raw) return null;
  const obj = tryParseJson(raw);

  // shell_exec: show stderr (red) + stdout, only if non-empty
  if (obj && name === 'shell_exec') {
    const parts: FormattedResult['parts'] = [];
    const stderr = typeof obj.stderr === 'string' ? obj.stderr : '';
    const stdout = typeof obj.stdout === 'string' ? obj.stdout : '';

    if (stderr) parts.push({ text: stderr, isStderr: true });
    if (stdout) parts.push({ text: stdout, isStderr: false });

    if (parts.length > 0) return { parts };
    // If both empty, fall through to raw display
  }

  // Default: show raw result
  return { parts: [{ text: raw, isStderr: false }] };
}

export function ToolCallCard({ toolCall, status = 'success', result, durationMs }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);

  const statusIcon = {
    running: <Loader size={13} className="tool-status-spinner" />,
    success: <CheckCircle size={13} />,
    error: <XCircle size={13} />,
  }[status];

  const statusLabel = {
    running: 'Running…',
    success: 'Done',
    error: 'Failed',
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

  const hasExpandable = displayArgs || displayResult;

  return (
    <div className={`tool-call-card ${statusClass}`}>
      <div className="tool-call-header" onClick={() => hasExpandable && setExpanded(!expanded)}>
        <span className="tool-call-icon"><Wrench size={12} /></span>
        <span className="tool-call-name">{toolCall.name}</span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        <span className={`tool-call-status ${statusClass}`}>{statusLabel}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{durationMs}ms</span>
        )}
        {hasExpandable && (
          <span className={`tool-call-expand ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-body">
          {displayArgs && (
            <div className="tool-call-section">
              <div className="tool-call-label">Arguments</div>
              <pre className="tool-call-code">{displayArgs}</pre>
            </div>
          )}
          {displayResult && (
            <div className="tool-call-section">
              <div className="tool-call-label">Result</div>
              <pre className="tool-call-code">
                {displayResult.parts.map((part, i) => (
                  <span key={i} className={part.isStderr ? 'tool-result-stderr' : ''}>
                    {part.text}
                    {i < displayResult.parts.length - 1 ? '\n' : ''}
                  </span>
                ))}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
