import { useState } from 'react';
import { ChevronDown, ChevronRight, Copy } from 'lucide-react';
import type { ExecutionRecord } from './types';

export function ExecutionHistorySection({ executions }: { executions: ExecutionRecord[] }) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [copiedField, setCopiedField] = useState<string | null>(null);

  const handleCopy = async (text: string, fieldId: string) => {
    try {
      if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
        await navigator.clipboard.writeText(text);
      } else {
        // Fallback for insecure contexts.
        const ta = document.createElement('textarea');
        ta.value = text;
        ta.style.position = 'fixed';
        ta.style.left = '-9999px';
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        document.body.removeChild(ta);
      }
      setCopiedField(fieldId);
      setTimeout(() => setCopiedField(null), 1500);
    } catch (e) {
      console.error('Copy failed:', e);
    }
  };

  if (executions.length === 0) {
    return (
      <div className="exec-history-empty">
        No executions yet. Click "Trigger Now" to run this schedule.
      </div>
    );
  }

  return (
    <div className="exec-history-list">
      {executions.map((exec) => {
        const isExpanded = expandedId === exec.execution_id;
        const statusClass = `exec-status--${exec.status}`;
        const triggeredDate = new Date(exec.triggered_at);

        return (
          <div key={exec.execution_id} className="exec-history-entry">
            <button
              className="exec-history-header"
              onClick={() => setExpandedId(isExpanded ? null : exec.execution_id)}
            >
              <span className="exec-history-chevron">
                {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              </span>
              <span className={`exec-status-badge ${statusClass}`}>
                {exec.status}
              </span>
              <span className="exec-history-time">
                {triggeredDate.toLocaleString()}
              </span>
              {exec.duration_ms !== null && (
                <span className="exec-history-duration">
                  {exec.duration_ms < 1000
                    ? `${exec.duration_ms}ms`
                    : `${(exec.duration_ms / 1000).toFixed(1)}s`}
                </span>
              )}
              <span className="exec-history-id">
                {exec.execution_id.length > 20
                  ? `...${exec.execution_id.slice(-12)}`
                  : exec.execution_id}
              </span>
            </button>

            {isExpanded && (
              <div className="exec-history-detail">
                {/* Error message */}
                {exec.error_message && (
                  <div className="exec-detail-error">
                    <span className="exec-detail-label">Error</span>
                    <div className="exec-detail-error-message">{exec.error_message}</div>
                  </div>
                )}

                {/* Request summary */}
                <div className="exec-detail-section">
                  <div className="exec-detail-section-header">
                    <span className="exec-detail-label">Request</span>
                    <button
                      className="exec-copy-btn"
                      onClick={() => handleCopy(
                        JSON.stringify(exec.request_summary, null, 2),
                        `req-${exec.execution_id}`,
                      )}
                    >
                      <Copy size={12} />
                      {copiedField === `req-${exec.execution_id}` ? 'Copied' : 'Copy'}
                    </button>
                  </div>
                  <pre className="exec-detail-json">
                    {JSON.stringify(exec.request_summary, null, 2)}
                  </pre>
                </div>

                {/* Response summary */}
                <div className="exec-detail-section">
                  <div className="exec-detail-section-header">
                    <span className="exec-detail-label">Response</span>
                    <button
                      className="exec-copy-btn"
                      onClick={() => handleCopy(
                        JSON.stringify(exec.response_summary, null, 2),
                        `res-${exec.execution_id}`,
                      )}
                    >
                      <Copy size={12} />
                      {copiedField === `res-${exec.execution_id}` ? 'Copied' : 'Copy'}
                    </button>
                  </div>
                  <pre className="exec-detail-json">
                    {JSON.stringify(exec.response_summary, null, 2)}
                  </pre>
                </div>

                {/* Additional metadata */}
                <div className="exec-detail-meta">
                  {exec.workflow_execution_id && (
                    <span className="exec-detail-meta-item">
                      Workflow: <code>{exec.workflow_execution_id}</code>
                    </span>
                  )}
                  {exec.started_at && (
                    <span className="exec-detail-meta-item">
                      Started: {new Date(exec.started_at).toLocaleTimeString()}
                    </span>
                  )}
                  {exec.completed_at && (
                    <span className="exec-detail-meta-item">
                      Completed: {new Date(exec.completed_at).toLocaleTimeString()}
                    </span>
                  )}
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
