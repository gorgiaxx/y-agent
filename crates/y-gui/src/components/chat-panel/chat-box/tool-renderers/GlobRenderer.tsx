import { useState, useMemo } from 'react';
import { FolderSearch, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractGlobMeta, parseGlobResult, basename } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function GlobRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const globMeta = extractGlobMeta(toolCall.name, toolCall.arguments);
  const globResult = useMemo(
    () => (result ? parseGlobResult(result) : null),
    [result],
  );

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = !!globResult || hasExpandable;

  if (!globMeta) {
    return null;
  }

  return (
    <div className={`tool-call-file-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={globMeta.searchPath ? `Glob: ${globMeta.pattern} in ${globMeta.searchPath}` : `Glob: ${globMeta.pattern}`}
      >
        <span className="tool-call-action-group">
          <FolderSearch size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">Glob</span>
        </span>
        <span className="tool-call-monospace-value">{globMeta.pattern}</span>
        {globResult && (
          <span className="tool-call-glob-count">{globResult.count} files</span>
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
          {globResult ? (
            <>
              <div className="tool-call-glob-summary">
                <span className="tool-call-glob-summary-count">{globResult.count} matches</span>
                {globMeta.searchPath && (
                  <span className="tool-call-glob-summary-path">in {globMeta.searchPath}</span>
                )}
                {globResult.truncated && (
                  <span className="tool-call-glob-truncated">truncated</span>
                )}
              </div>
              <div className="tool-call-glob-matches">
                {globResult.matches.map((m, i) => (
                  <span key={i} className="tool-call-glob-match" title={m}>{basename(m)}</span>
                ))}
              </div>
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
