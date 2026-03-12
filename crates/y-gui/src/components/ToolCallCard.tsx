import { useState } from 'react';
import type { ToolCallBrief } from '../types';
import './ToolCallCard.css';

interface ToolCallCardProps {
  toolCall: ToolCallBrief;
  status?: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
}

export function ToolCallCard({ toolCall, status = 'success', result, durationMs }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);

  const statusIcon = {
    running: '⏳',
    success: '✓',
    error: '✗',
  }[status];

  const statusClass = `tool-status-${status}`;

  return (
    <div className={`tool-call-card ${statusClass}`}>
      <div className="tool-call-header" onClick={() => setExpanded(!expanded)}>
        <span className="tool-call-icon">🔧</span>
        <span className="tool-call-name">{toolCall.name}</span>
        <span className={`tool-call-status ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{durationMs}ms</span>
        )}
        <span className={`tool-call-expand ${expanded ? 'expanded' : ''}`}>▸</span>
      </div>
      {expanded && (
        <div className="tool-call-body">
          {toolCall.arguments && (
            <div className="tool-call-section">
              <div className="tool-call-label">Arguments</div>
              <pre className="tool-call-code">{toolCall.arguments}</pre>
            </div>
          )}
          {result && (
            <div className="tool-call-section">
              <div className="tool-call-label">Result</div>
              <pre className="tool-call-code">{result}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
