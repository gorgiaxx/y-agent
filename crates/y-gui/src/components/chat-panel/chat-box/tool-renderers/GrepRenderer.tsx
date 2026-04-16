import { useState, useMemo } from 'react';
import { Search, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractGrepMeta, parseGrepResult, basename } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function GrepRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const grepMeta = extractGrepMeta(toolCall.name, toolCall.arguments);
  const grepResult = useMemo(
    () => (result ? parseGrepResult(result) : null),
    [result],
  );

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = !!grepResult || hasExpandable;

  if (!grepMeta) {
    return null;
  }

  const getSummaryText = () => {
    if (!grepResult) return null;
    switch (grepResult.mode) {
      case 'files_with_matches':
        return `${grepResult.numFiles} files`;
      case 'count':
        return `${grepResult.numMatches} matches in ${grepResult.numFiles} files`;
      case 'content':
        return `${grepResult.numLines} lines in ${grepResult.numFiles} files`;
      default:
        return null;
    }
  };

  return (
    <div className={`tool-call-file-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={grepMeta.path ? `Grep: ${grepMeta.pattern} in ${grepMeta.path}` : `Grep: ${grepMeta.pattern}`}
      >
        <span className="tool-call-action-group">
          <Search size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">Grep</span>
        </span>
        <span className="tool-call-monospace-value">{grepMeta.pattern}</span>
        {grepResult && (
          <span className="tool-call-glob-count">{getSummaryText()}</span>
        )}
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        {canExpand && (
          <span className={`tool-call-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-glob-detail">
          {grepResult ? (
            <>
              <div className="tool-call-glob-summary">
                <span className="tool-call-glob-summary-count">{getSummaryText()}</span>
                {grepMeta.path && (
                  <span className="tool-call-glob-summary-path">in {grepMeta.path}</span>
                )}
                {grepMeta.glob && (
                  <span className="tool-call-grep-filter">glob: {grepMeta.glob}</span>
                )}
                {grepMeta.type && (
                  <span className="tool-call-grep-filter">type: {grepMeta.type}</span>
                )}
                {grepMeta.caseInsensitive && (
                  <span className="tool-call-grep-filter">-i</span>
                )}
                {grepResult.truncated && (
                  <span className="tool-call-glob-truncated">truncated</span>
                )}
              </div>
              {grepResult.mode === 'files_with_matches' && grepResult.filenames && (
                <div className="tool-call-glob-matches">
                  {grepResult.filenames.map((f, i) => (
                    <span key={i} className="tool-call-glob-match" title={f}>{basename(f)}</span>
                  ))}
                </div>
              )}
              {(grepResult.mode === 'content' || grepResult.mode === 'count') && grepResult.content && (
                <pre className="tool-call-grep-content">{grepResult.content}</pre>
              )}
            </>
          ) : (
            <DetailSections
              displayResult={activeResult}
              showRaw={showRaw}
              onToggleRaw={() => setShowRaw(!showRaw)}
            />
          )}
        </div>
      )}
    </div>
  );
}
