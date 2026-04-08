import { useState, useMemo } from 'react';
import { ExternalLink } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { formatDuration } from '../../../../utils/formatDuration';
import { tryParseJson, extractDomain, extractUrlMeta } from '../toolCallUtils';
import { Favicon, DetailSections } from './shared';
import { DefaultRenderer } from './DefaultRenderer';
import type { ToolRendererProps } from './types';

export function UrlRenderer(props: ToolRendererProps) {
  const {
    toolCall, durationMs,
    statusIcon, statusClass,
    displayArgs, displayResult, displayResultFormatted,
    urlMeta: urlMetaProp, result,
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
  }, [urlMetaProp, toolCall.name, toolCall.arguments, result]);

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  if (!urlMeta) {
    return <DefaultRenderer {...props} />;
  }

  const displayTitle = urlMeta.title || urlMeta.domain;

  const handleOpenExternal = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    openUrl(urlMeta.url).catch((err) =>
      console.error('[ToolCallCard] failed to open URL:', urlMeta.url, err),
    );
  };

  return (
    <div className={`tool-call-url-wrapper ${statusClass}`}>
      <div
        className="tool-call-url-tag"
        onClick={() => hasExpandable && setExpanded(!expanded)}
        title={urlMeta.url}
      >
        <Favicon faviconUrl={urlMeta.faviconUrl} />
        <span className="tool-call-url-title">{displayTitle}</span>
        {urlMeta.title && (
          <span className="tool-call-url-domain">{urlMeta.domain}</span>
        )}
        <button
          className="tool-call-url-open"
          onClick={handleOpenExternal}
          title="Open in browser"
          aria-label="Open URL in external browser"
        >
          <ExternalLink size={12} />
        </button>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
      </div>
      {expanded && hasExpandable && (
        <div className="tool-call-url-detail">
          <DetailSections
            displayArgs={displayArgs}
            displayResult={activeResult}
            showRaw={showRaw}
            onToggleRaw={() => setShowRaw(!showRaw)}
          />
        </div>
      )}
    </div>
  );
}
