import { useState, useMemo } from 'react';
import { Search, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractToolSearchMeta, formatToolSearchResult } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function ToolSearchRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const searchMeta = extractToolSearchMeta(toolCall.arguments);
  const searchResult = useMemo(
    () => (result ? formatToolSearchResult(result) : null),
    [result],
  );

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = !!searchMeta || displayResult;
  const canExpand = !!searchResult || hasExpandable;

  if (!searchMeta) {
    return null;
  }

  return (
    <div className={`tool-call-search-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={`ToolSearch: ${searchMeta.key}=${searchMeta.value}`}
      >
        <Search size={14} className="tool-call-icon-muted" />
        <span className="tool-call-key">{searchMeta.key}:</span>
        <span className="tool-call-monospace-value">{searchMeta.value}</span>
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
        <div className="tool-call-detail">
          {searchResult ? (
            searchResult.lines.map((group, gi) => (
              <div key={gi} className="tool-call-search-group">
                <div className="tool-call-search-group-label">{group.label}</div>
                <div className="tool-call-search-group-items">
                  {group.items.map((item, ii) => (
                    <span key={ii} className="tool-call-search-item">{item}</span>
                  ))}
                </div>
              </div>
            ))
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
