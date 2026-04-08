import { useState, useMemo } from 'react';
import { Wrench, CheckCircle, XCircle, Loader, Globe, ExternalLink, SquareTerminal, ChevronRight, FilePenLine, FilePlus2, FileSearch, FolderSearch, Search, Code, FileText } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { ToolCallBrief } from '../../../types';
import { CollapsibleCard } from './CollapsibleCard';
import { formatDuration } from '../../../utils/formatDuration';
import {
  tryParseJson,
  extractDomain,
  basename,
  extractUrlMeta,
  extractShellCommand,
  extractToolSearchMeta,
  formatToolSearchResult,
  extractGrepMeta,
  parseGrepResult,
  extractGlobMeta,
  parseGlobResult,
  extractFileToolMeta,
  computeLineDiff,
  formatArguments,
  formatResult,
  formatResultFormatted,
} from './toolCallUtils';
import type { FileToolType } from './toolCallUtils';
import './ToolCallCard.css';

interface ToolCallCardProps {
  toolCall: ToolCallBrief;
  status?: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
  /** Compact URL metadata JSON from the backend (survives truncation). */
  urlMeta?: string;
}

// ---------------------------------------------------------------------------
// Favicon component with fallback chain
// ---------------------------------------------------------------------------

function Favicon({ faviconUrl }: { faviconUrl: string }) {
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

function FileDiffView({ oldString, newString }: { oldString: string; newString: string }) {
  const diffLines = useMemo(
    () => computeLineDiff(oldString, newString),
    [oldString, newString],
  );

  // Compute summary: count added/removed lines
  const addCount = diffLines.filter(l => l.type === 'add').length;
  const removeCount = diffLines.filter(l => l.type === 'remove').length;

  if (diffLines.length === 0) {
    return <div className="tool-call-diff-empty">No changes</div>;
  }

  // Figure out the max line number width for gutter alignment
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

function DetailSections({
  displayArgs,
  displayResult,
  argsLabel = 'Arguments',
  resultLabel = 'Result',
  showRaw,
  onToggleRaw,
}: {
  displayArgs?: string;
  displayResult?: { parts: Array<{ text: string; isStderr: boolean }> } | null;
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

// ---------------------------------------------------------------------------
// ToolCallCard
// ---------------------------------------------------------------------------

const ACCENT_COLOR = 'var(--accent)';

export function ToolCallCard({ toolCall, status = 'success', result, durationMs, urlMeta: urlMetaProp }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  // Prefer the dedicated urlMeta prop (survives truncation) over parsing result.
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

  const statusIcon = {
    running: <Loader size={13} className="collapsible-card-spinner" />,
    success: <CheckCircle size={13} />,
    error: <XCircle size={13} />,
  }[status];

  const statusLabel = {
    running: 'Running...',
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

  const displayResultFormatted = useMemo(
    () => (result ? formatResultFormatted(toolCall.name, result, toolCall.arguments) : null),
    [toolCall.name, result, toolCall.arguments],
  );

  // Use formatted result by default, raw when toggled
  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);

  const hasExpandable = displayArgs || displayResult;

  // Pre-compute all tool-specific metadata and memoised results at the top
  // to avoid calling hooks conditionally (Rules of Hooks).
  const shellCommand = toolCall.name === 'ShellExec' ? extractShellCommand(toolCall.arguments) : null;
  const searchMeta = toolCall.name === 'ToolSearch' ? extractToolSearchMeta(toolCall.arguments) : null;
  const searchResult = useMemo(
    () => (result ? formatToolSearchResult(result) : null),
    [result],
  );
  const globMeta = extractGlobMeta(toolCall.name, toolCall.arguments);
  const globResult = useMemo(
    () => (result ? parseGlobResult(result) : null),
    [result],
  );
  const grepMeta = extractGrepMeta(toolCall.name, toolCall.arguments);
  const grepResult = useMemo(
    () => (result ? parseGrepResult(result) : null),
    [result],
  );
  const fileMeta = extractFileToolMeta(toolCall.name, toolCall.arguments);

  // ---- URL tag rendering for Browser/WebFetch ----
  if (urlMeta) {
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
            <DetailSections displayArgs={displayArgs} displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
          </div>
        )}
      </div>
    );
  }

  // ---- ShellExec inline tag rendering ----
  if (shellCommand !== null) {
    return (
      <div className={`tool-call-shell-wrapper ${statusClass}`}>
        <div
          className="tool-call-shell-tag"
          onClick={() => hasExpandable && setExpanded(!expanded)}
          title={shellCommand}
        >
          <SquareTerminal size={14} className="tool-call-shell-icon" />
          <span className="tool-call-shell-command">{shellCommand}</span>
          <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
          {durationMs !== undefined && (
            <span className="tool-call-duration">{formatDuration(durationMs)}</span>
          )}
          <span className={`tool-call-shell-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        </div>
        {expanded && hasExpandable && (
          <div className="tool-call-shell-detail">
            <DetailSections displayArgs={displayArgs} displayResult={activeResult} argsLabel="Command" resultLabel="Output" showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
          </div>
        )}
      </div>
    );
  }

  // ---- ToolSearch compact tag rendering ----
  if (searchMeta) {
    const canExpand = !!searchResult || hasExpandable;

    return (
      <div className={`tool-call-search-wrapper ${statusClass}`}>
        <div
          className="tool-call-search-tag"
          onClick={() => canExpand && setExpanded(!expanded)}
          title={`ToolSearch: ${searchMeta.key}=${searchMeta.value}`}
        >
          <Search size={14} className="tool-call-search-icon" />
          <span className="tool-call-search-key">{searchMeta.key}:</span>
          <span className="tool-call-search-value">{searchMeta.value}</span>
          <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
          {durationMs !== undefined && (
            <span className="tool-call-duration">{formatDuration(durationMs)}</span>
          )}
          {canExpand && (
            <span className={`tool-call-search-chevron ${expanded ? 'expanded' : ''}`}>
              <ChevronRight size={12} />
            </span>
          )}
        </div>
        {expanded && (
          <div className="tool-call-search-detail">
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
              <DetailSections displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
            )}
          </div>
        )}
      </div>
    );
  }

  // ---- Glob compact tag rendering ----
  if (globMeta) {
    const canExpand = !!globResult || hasExpandable;

    return (
      <div className={`tool-call-file-wrapper ${statusClass}`}>
        <div
          className="tool-call-file-tag"
          onClick={() => canExpand && setExpanded(!expanded)}
          title={globMeta.searchPath ? `Glob: ${globMeta.pattern} in ${globMeta.searchPath}` : `Glob: ${globMeta.pattern}`}
        >
          <span className="tool-call-file-action-group">
            <FolderSearch size={14} className="tool-call-file-icon" />
            <span className="tool-call-file-action">Glob</span>
          </span>
          <span className="tool-call-file-name">{globMeta.pattern}</span>
          {globResult && (
            <span className="tool-call-glob-count">{globResult.count} files</span>
          )}
          <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
          {durationMs !== undefined && (
            <span className="tool-call-duration">{formatDuration(durationMs)}</span>
          )}
          {canExpand && (
            <span className={`tool-call-file-chevron ${expanded ? 'expanded' : ''}`}>
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
              <DetailSections displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
            )}
          </div>
        )}
      </div>
    );
  }

  // ---- Grep compact tag rendering ----
  if (grepMeta) {
    const canExpand = !!grepResult || hasExpandable;

    // Build summary text based on mode
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
          className="tool-call-file-tag"
          onClick={() => canExpand && setExpanded(!expanded)}
          title={grepMeta.path ? `Grep: ${grepMeta.pattern} in ${grepMeta.path}` : `Grep: ${grepMeta.pattern}`}
        >
          <span className="tool-call-file-action-group">
            <Search size={14} className="tool-call-file-icon" />
            <span className="tool-call-file-action">Grep</span>
          </span>
          <span className="tool-call-file-name">{grepMeta.pattern}</span>
          {grepResult && (
            <span className="tool-call-glob-count">{getSummaryText()}</span>
          )}
          <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
          {durationMs !== undefined && (
            <span className="tool-call-duration">{formatDuration(durationMs)}</span>
          )}
          {canExpand && (
            <span className={`tool-call-file-chevron ${expanded ? 'expanded' : ''}`}>
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
              <DetailSections displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
            )}
          </div>
        )}
      </div>
    );
  }

  // ---- FileEdit / FileWrite / FileRead inline tag rendering ----
  // Also handle cases where fileMeta extraction fails (e.g. malformed args)
  // but the tool name is still a file tool -- render as file-tag with fallback.
  const isFileTool = toolCall.name === 'FileEdit' || toolCall.name === 'FileWrite' || toolCall.name === 'FileRead';
  if (fileMeta || isFileTool) {
    const FILE_ICONS: Record<FileToolType, typeof FilePenLine> = {
      read: FileSearch,
      edit: FilePenLine,
      write: FilePlus2,
    };
    const FILE_LABELS: Record<FileToolType, string> = {
      read: 'Read',
      edit: 'Update',
      write: 'Create',
    };
    const fallbackType: FileToolType = toolCall.name === 'FileEdit' ? 'edit'
      : toolCall.name === 'FileRead' ? 'read' : 'write';
    const toolType = fileMeta?.toolType ?? fallbackType;
    const fileName = fileMeta ? basename(fileMeta.filePath) : toolCall.name;
    const FileIcon = FILE_ICONS[toolType];
    const fileLabel = FILE_LABELS[toolType];
    const hasDiff = fileMeta?.toolType === 'edit' && fileMeta.oldString !== undefined && fileMeta.newString !== undefined;
    // When failed, show error result instead of diff
    const showDiff = hasDiff && status !== 'error';
    // On error, always allow expansion so the error detail is visible
    const canExpand = showDiff || hasExpandable || status === 'error';

    return (
      <div className={`tool-call-file-wrapper ${statusClass}`}>
        <div
          className="tool-call-file-tag"
          onClick={() => canExpand && setExpanded(!expanded)}
          title={fileMeta?.filePath ?? toolCall.name}
        >
          <span className="tool-call-file-action-group">
            <FileIcon size={14} className="tool-call-file-icon" />
            <span className="tool-call-file-action">{fileLabel}</span>
          </span>
          <span className="tool-call-file-name">{fileName}</span>
          <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
          {durationMs !== undefined && (
            <span className="tool-call-duration">{formatDuration(durationMs)}</span>
          )}
          {canExpand && (
            <span className={`tool-call-file-chevron ${expanded ? 'expanded' : ''}`}>
              <ChevronRight size={12} />
            </span>
          )}
        </div>
        {expanded && (
          <div className="tool-call-file-detail">
            {showDiff && <FileDiffView oldString={fileMeta!.oldString!} newString={fileMeta!.newString!} />}
            {!showDiff && <DetailSections displayArgs={displayArgs} displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />}
          </div>
        )}
      </div>
    );
  }

  // ---- Default rendering for all other tools ----
  const headerRight = (
    <>
      <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
      <span className={`tool-call-status ${statusClass}`}>{statusLabel}</span>
      {durationMs !== undefined && (
        <span className="tool-call-duration">{formatDuration(durationMs)}</span>
      )}
    </>
  );

  return (
    <CollapsibleCard
      icon={<Wrench size={12} />}
      label={<span className="tool-call-name">{toolCall.name}</span>}
      accentColor={ACCENT_COLOR}
      expanded={expanded}
      onToggle={() => hasExpandable && setExpanded(!expanded)}
      headerRight={headerRight}
      className="tool-call-card"
    >
      <DetailSections displayArgs={displayArgs} displayResult={activeResult} showRaw={showRaw} onToggleRaw={() => setShowRaw(!showRaw)} />
    </CollapsibleCard>
  );
}
