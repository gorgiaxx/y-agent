import { useState, useMemo } from 'react';
import { BookOpen, ChevronRight } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { extractKnowledgeSearchMeta, parseKnowledgeSearchResult } from '../toolCallUtils';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

export function KnowledgeSearchRenderer({
  toolCall, durationMs, result,
  statusIcon, statusClass,
  displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const meta = extractKnowledgeSearchMeta(toolCall.arguments);
  const searchResult = useMemo(
    () => (result ? parseKnowledgeSearchResult(result) : null),
    [result],
  );

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = !!meta || displayResult;
  const canExpand = !!searchResult || hasExpandable;

  if (!meta) {
    return null;
  }

  const summaryText = searchResult
    ? `${searchResult.count} result${searchResult.count !== 1 ? 's' : ''}`
    : null;

  return (
    <div className={`tool-call-file-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={meta.domain
          ? `KnowledgeSearch: ${meta.query} (domain: ${meta.domain})`
          : `KnowledgeSearch: ${meta.query}`}
      >
        <span className="tool-call-action-group">
          <BookOpen size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">Knowledge</span>
        </span>
        <span className="tool-call-monospace-value">{meta.query}</span>
        {meta.domain && (
          <span className="tool-call-grep-filter">{meta.domain}</span>
        )}
        {searchResult && (
          <span className="tool-call-glob-count">{summaryText}</span>
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
          {searchResult ? (
            <>
              <div className="tool-call-glob-summary">
                <span className="tool-call-glob-summary-count">{summaryText}</span>
                {searchResult.truncated && (
                  <span className="tool-call-glob-truncated">truncated</span>
                )}
              </div>
              {searchResult.results.length > 0 && (
                <div className="tool-call-glob-matches">
                  {searchResult.results.map((item, i) => (
                    <span
                      key={i}
                      className="tool-call-glob-match"
                      title={`relevance: ${item.relevance}`}
                    >
                      {item.title}
                    </span>
                  ))}
                </div>
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
