import { useMemo, useState } from 'react';
import { Globe } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import {
  extractBrowserActionSummary,
  extractDomain,
  extractUrlMeta,
  tryParseJson,
} from '../toolCallUtils';
import { DetailSections } from './shared';
import { UrlRenderer } from './UrlRenderer';
import type { ToolRendererProps } from './types';

export function BrowserRenderer(props: ToolRendererProps) {
  const {
    toolCall,
    durationMs,
    statusIcon,
    statusClass,
    displayArgs,
    displayResult,
    displayResultFormatted,
    urlMeta: urlMetaProp,
    result,
  } = props;

  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const urlMeta = useMemo(() => {
    if (urlMetaProp) {
      const parsed = tryParseJson(urlMetaProp);
      if (parsed && typeof parsed.url === 'string' && parsed.url) {
        return {
          url: parsed.url as string,
          title: String(parsed.title ?? ''),
          faviconUrl: String(parsed.favicon_url ?? ''),
          domain: extractDomain(parsed.url as string),
        };
      }
    }

    return extractUrlMeta(toolCall.name, toolCall.arguments, result);
  }, [toolCall.arguments, toolCall.name, result, urlMetaProp]);

  const actionSummary = useMemo(
    () => extractBrowserActionSummary(toolCall.arguments, result),
    [toolCall.arguments, result],
  );
  const activeResult = showRaw ? displayResult : displayResultFormatted;
  const hasExpandable = Boolean(displayArgs || displayResult);
  const label = actionSummary?.label ?? 'Browser';
  const detail = actionSummary?.detail;

  if (urlMeta) {
    return <UrlRenderer {...props} />;
  }

  return (
    <div className={`tool-call-url-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={detail ?? label}
      >
        <Globe size={14} className="tool-call-icon-muted" />
        <span className="tool-call-url-title">{label}</span>
        {detail && (
          <span className="tool-call-url-domain">{detail}</span>
        )}
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
      </div>
      {expanded && hasExpandable && (
        <div className="tool-call-detail">
          <DetailSections
            displayArgs={displayArgs}
            displayResult={activeResult}
            showRaw={showRaw}
            onToggleRaw={displayResult ? () => setShowRaw(!showRaw) : undefined}
          />
        </div>
      )}
    </div>
  );
}
