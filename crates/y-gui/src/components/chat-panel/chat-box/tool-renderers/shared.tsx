import { useState, useMemo } from 'react';
import { Globe, Code, FileText } from 'lucide-react';
import { computeLineDiff } from '../toolCallUtils';
import type { FormattedResult } from '../toolCallUtils';

// ---------------------------------------------------------------------------
// Favicon component with fallback chain
// ---------------------------------------------------------------------------

export function Favicon({ faviconUrl }: { faviconUrl: string }) {
  const [failed, setFailed] = useState(false);

  if (!faviconUrl || failed) {
    return <Globe size={14} className="tool-call-url-favicon tool-call-url-favicon--icon" />;
  }

  return (
    <img
      src={faviconUrl}
      width={14}
      height={14}
      alt=""
      className="tool-call-url-favicon"
      onError={() => setFailed(true)}
    />
  );
}

// ---------------------------------------------------------------------------
// FileDiffView -- inline diff for FileEdit tool calls
// ---------------------------------------------------------------------------

export function FileDiffView({ oldString, newString }: { oldString: string; newString: string }) {
  const diffLines = useMemo(
    () => computeLineDiff(oldString, newString),
    [oldString, newString],
  );

  const addCount = diffLines.filter(l => l.type === 'add').length;
  const removeCount = diffLines.filter(l => l.type === 'remove').length;

  if (diffLines.length === 0) {
    return <div className="tool-call-diff-empty">No changes</div>;
  }

  let maxLineNo = 1;
  for (const line of diffLines) {
    if (line.oldLineNo && line.oldLineNo > maxLineNo) maxLineNo = line.oldLineNo;
    if (line.newLineNo && line.newLineNo > maxLineNo) maxLineNo = line.newLineNo;
  }
  const gutterWidth = Math.max(String(maxLineNo).length, 2);

  return (
    <div className="tool-call-diff">
      <div className="tool-call-diff-summary">
        {addCount > 0 && <span className="tool-call-diff-stat-add">+{addCount}</span>}
        {removeCount > 0 && <span className="tool-call-diff-stat-remove">-{removeCount}</span>}
      </div>
      {diffLines.map((line, i) => {
        if (line.type === 'separator') {
          return (
            <div key={i} className="tool-call-diff-line tool-call-diff-separator">
              <span className="tool-call-diff-gutter" style={{ width: `${gutterWidth * 2 + 3}ch` }}>...</span>
              <span className="tool-call-diff-marker"> </span>
              <span className="tool-call-diff-text"></span>
            </div>
          );
        }

        const oldNo = line.oldLineNo != null ? String(line.oldLineNo).padStart(gutterWidth) : ' '.repeat(gutterWidth);
        const newNo = line.newLineNo != null ? String(line.newLineNo).padStart(gutterWidth) : ' '.repeat(gutterWidth);
        const marker = line.type === 'add' ? '+' : line.type === 'remove' ? '-' : ' ';

        return (
          <div
            key={i}
            className={`tool-call-diff-line tool-call-diff-${line.type}`}
          >
            <span className="tool-call-diff-gutter" style={{ width: `${gutterWidth}ch` }}>{oldNo}</span>
            <span className="tool-call-diff-gutter" style={{ width: `${gutterWidth}ch` }}>{newNo}</span>
            <span className="tool-call-diff-marker">{marker}</span>
            <span className="tool-call-diff-text">{line.text}</span>
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shared detail section renderer
// ---------------------------------------------------------------------------

export function DetailSections({
  displayArgs,
  displayResult,
  argsLabel = 'Arguments',
  resultLabel = 'Result',
  showRaw,
  onToggleRaw,
}: {
  displayArgs?: string;
  displayResult?: FormattedResult | null;
  argsLabel?: string;
  resultLabel?: string;
  showRaw?: boolean;
  onToggleRaw?: () => void;
}) {
  return (
    <>
      {displayArgs && (
        <div className="tool-call-section">
          <div className="tool-call-label">{argsLabel}</div>
          <pre className="tool-call-code">{displayArgs}</pre>
        </div>
      )}
      {displayResult && (
        <div className="tool-call-section">
          <div className="tool-call-label-row">
            <div className="tool-call-label">{resultLabel}</div>
            {onToggleRaw && (
              <button
                className={`tool-call-raw-toggle ${showRaw ? 'active' : ''}`}
                onClick={(e) => { e.stopPropagation(); onToggleRaw(); }}
                title={showRaw ? 'Show formatted' : 'Show raw JSON'}
              >
                {showRaw ? <FileText size={11} /> : <Code size={11} />}
                <span>{showRaw ? 'Formatted' : 'Raw'}</span>
              </button>
            )}
          </div>
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
    </>
  );
}
