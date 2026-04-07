import { useState, useMemo } from 'react';
import { Wrench, CheckCircle, XCircle, Loader, Globe, ExternalLink, SquareTerminal, ChevronRight, FilePenLine, FilePlus2, FileSearch, FolderSearch, Search } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { ToolCallBrief } from '../../../types';
import { CollapsibleCard } from './CollapsibleCard';
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
// URL metadata extraction for Browser/WebFetch tool calls
// ---------------------------------------------------------------------------

interface UrlToolMeta {
  url: string;
  title: string;
  faviconUrl: string;
  domain: string;
}

/** Extract URL metadata from Browser/WebFetch tool calls. */
function extractUrlMeta(
  toolName: string,
  argsRaw: string,
  resultRaw?: string,
): UrlToolMeta | null {
  const parsedArgs = tryParseJson(argsRaw);
  const parsedResult = resultRaw ? tryParseJson(resultRaw) : null;

  if (toolName === 'Browser') {
    // Detect navigate/search from arguments or from result action field
    const action = String(parsedArgs?.action ?? parsedResult?.action ?? '');
    if (action === 'navigate' || action === 'search') {
      const url = String(parsedResult?.url ?? parsedArgs?.url ?? parsedArgs?.query ?? '');
      if (!url) return null;
      return {
        url,
        title: String(parsedResult?.title ?? ''),
        faviconUrl: String(parsedResult?.favicon_url ?? ''),
        domain: extractDomain(url),
      };
    }
    // Fallback: result has a url field (enriched navigate response)
    if (parsedResult?.url && typeof parsedResult.url === 'string') {
      return {
        url: parsedResult.url as string,
        title: String(parsedResult?.title ?? ''),
        faviconUrl: String(parsedResult?.favicon_url ?? ''),
        domain: extractDomain(parsedResult.url as string),
      };
    }
  }

  if (toolName === 'WebFetch') {
    const url = String(parsedResult?.url ?? parsedArgs?.url ?? parsedArgs?.query ?? '');
    if (!url) return null;
    return {
      url,
      title: String(parsedResult?.title ?? ''),
      faviconUrl: String(parsedResult?.favicon_url ?? ''),
      domain: extractDomain(url),
    };
  }

  return null;
}

/** Extract hostname from a URL, returning the raw string on failure. */
function extractDomain(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

// ---------------------------------------------------------------------------
// Smart formatting helpers
// ---------------------------------------------------------------------------

/** Try to parse JSON; return null on failure. */
function tryParseJson(raw: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === 'object' && parsed !== null ? parsed : null;
  } catch {
    return null;
  }
}

/** Extract the command string from ShellExec arguments. */
function extractShellCommand(argsRaw: string): string | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  if (typeof obj.command === 'string') return obj.command;
  return null;
}

// ---------------------------------------------------------------------------
// ToolSearch helpers
// ---------------------------------------------------------------------------

interface ToolSearchMeta {
  /** The search mode key: 'query', 'category', or 'tool'. */
  key: string;
  /** The search value. */
  value: string;
}

/** Extract the search key:value from ToolSearch arguments. */
function extractToolSearchMeta(argsRaw: string): ToolSearchMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  // Precedence: tool > category > query (matches backend).
  if (typeof obj.tool === 'string' && obj.tool) return { key: 'tool', value: obj.tool };
  if (typeof obj.category === 'string' && obj.category) return { key: 'category', value: obj.category };
  if (typeof obj.query === 'string' && obj.query) return { key: 'query', value: obj.query };
  return null;
}

/** Format ToolSearch result for structured display. */
function formatToolSearchResult(raw: string): { lines: Array<{ label: string; items: string[] }> } | null {
  const obj = tryParseJson(raw);
  if (!obj) return null;
  const lines: Array<{ label: string; items: string[] }> = [];

  // Mode 1: keyword search -- { tools: { results, count, activated }, skills, agents, total_count }
  const toolsObj = obj.tools as Record<string, unknown> | undefined;
  if (toolsObj && typeof toolsObj === 'object' && 'results' in toolsObj) {
    const results = (toolsObj.results ?? []) as Array<Record<string, unknown>>;
    if (results.length > 0) {
      lines.push({
        label: `Tools (${results.length})`,
        items: results.map(r => String(r.name ?? r.description ?? '')),
      });
    }
    const skills = (obj.skills ?? []) as Array<Record<string, unknown>>;
    if (skills.length > 0) {
      lines.push({
        label: `Skills (${skills.length})`,
        items: skills.map(s => String(s.name ?? '')),
      });
    }
    const agents = (obj.agents ?? []) as Array<Record<string, unknown>>;
    if (agents.length > 0) {
      lines.push({
        label: `Agents (${agents.length})`,
        items: agents.map(a => String(a.name ?? a.id ?? '')),
      });
    }
    return { lines };
  }

  // Mode 2: browse_category -- { category, detail, tools, tool_definitions, activated }
  if (obj.category && Array.isArray(obj.tools)) {
    const tools = obj.tools as string[];
    if (tools.length > 0) {
      lines.push({ label: `Category: ${String(obj.category)}`, items: tools });
    }
    return { lines };
  }

  // Mode 3: get_tool -- { name, description, parameters, category }
  if (obj.name && obj.description) {
    lines.push({ label: String(obj.name), items: [String(obj.description)] });
    return { lines };
  }

  return null;
}

// ---------------------------------------------------------------------------
// Glob helpers
// ---------------------------------------------------------------------------

interface GlobMeta {
  pattern: string;
  searchPath?: string;
}

/** Extract glob metadata from Glob tool call arguments. */
function extractGlobMeta(toolName: string, argsRaw: string): GlobMeta | null {
  if (toolName !== 'Glob') return null;
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const pattern = typeof obj.pattern === 'string' ? obj.pattern : '';
  if (!pattern) return null;
  return {
    pattern,
    searchPath: typeof obj.path === 'string' ? obj.path : undefined,
  };
}

interface GlobResult {
  matches: string[];
  count: number;
  truncated: boolean;
}

/** Parse the structured Glob result JSON. */
function parseGlobResult(raw: string): GlobResult | null {
  const obj = tryParseJson(raw);
  if (!obj) return null;
  if (!Array.isArray(obj.matches)) return null;
  return {
    matches: (obj.matches as unknown[]).map(m => String(m)),
    count: typeof obj.count === 'number' ? obj.count : (obj.matches as unknown[]).length,
    truncated: obj.truncated === true,
  };
}

// ---------------------------------------------------------------------------
// FileRead/FileEdit/FileWrite helpers
// ---------------------------------------------------------------------------

type FileToolType = 'read' | 'edit' | 'write';

interface FileToolMeta {
  filePath: string;
  toolType: FileToolType;
  oldString?: string;
  newString?: string;
}

/** Extract file metadata from FileRead/FileEdit/FileWrite tool call arguments. */
function extractFileToolMeta(
  toolName: string,
  argsRaw: string,
): FileToolMeta | null {
  if (toolName !== 'FileEdit' && toolName !== 'FileWrite' && toolName !== 'FileRead') return null;
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;

  if (toolName === 'FileEdit') {
    const filePath = typeof obj.file_path === 'string' ? obj.file_path : '';
    if (!filePath) return null;
    return {
      filePath,
      toolType: 'edit',
      oldString: typeof obj.old_string === 'string' ? obj.old_string : undefined,
      newString: typeof obj.new_string === 'string' ? obj.new_string : undefined,
    };
  }

  // FileRead and FileWrite both use 'path'
  const filePath = typeof obj.path === 'string' ? obj.path : '';
  if (!filePath) return null;
  return { filePath, toolType: toolName === 'FileRead' ? 'read' : 'write' };
}

/** Extract the basename from a file path. */
function basename(filePath: string): string {
  const parts = filePath.replace(/\\/g, '/').split('/');
  return parts[parts.length - 1] || filePath;
}

/** Compute simple line diff between old and new strings. */
interface DiffLine {
  type: 'context' | 'add' | 'remove' | 'separator';
  text: string;
  /** Line number in the old file (1-based). Undefined for additions/separators. */
  oldLineNo?: number;
  /** Line number in the new file (1-based). Undefined for removals/separators. */
  newLineNo?: number;
}

function computeLineDiff(oldStr: string, newStr: string): DiffLine[] {
  const oldLines = oldStr.split('\n');
  const newLines = newStr.split('\n');

  // Simple LCS-based diff for reasonable-sized inputs
  const m = oldLines.length;
  const n = newLines.length;

  // Build LCS table
  const dp: number[][] = Array.from({ length: m + 1 }, () =>
    Array(n + 1).fill(0),
  );
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      if (oldLines[i - 1] === newLines[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
  }

  // Backtrack to produce raw diff with line number tracking
  let oi = m;
  let ni = n;
  interface RawDiffLine { type: 'context' | 'add' | 'remove'; text: string; oldLineNo?: number; newLineNo?: number }
  const stack: RawDiffLine[] = [];
  while (oi > 0 || ni > 0) {
    if (oi > 0 && ni > 0 && oldLines[oi - 1] === newLines[ni - 1]) {
      stack.push({ type: 'context', text: oldLines[oi - 1], oldLineNo: oi, newLineNo: ni });
      oi--;
      ni--;
    } else if (ni > 0 && (oi === 0 || dp[oi][ni - 1] >= dp[oi - 1][ni])) {
      stack.push({ type: 'add', text: newLines[ni - 1], newLineNo: ni });
      ni--;
    } else {
      stack.push({ type: 'remove', text: oldLines[oi - 1], oldLineNo: oi });
      oi--;
    }
  }
  stack.reverse();

  // Trim context: show at most 3 lines around changes, insert separators
  const CONTEXT = 3;
  const changeIndices = stack.map((l, idx) => (l.type !== 'context' ? idx : -1)).filter(x => x >= 0);
  if (changeIndices.length === 0) return stack;

  const result: DiffLine[] = [];
  let lastIncluded = -1;

  for (let si = 0; si < stack.length; si++) {
    const nearest = changeIndices.reduce((best, ci) =>
      Math.abs(ci - si) < Math.abs(best - si) ? ci : best,
    );
    if (stack[si].type !== 'context' || Math.abs(nearest - si) <= CONTEXT) {
      // If there's a gap since last included line, insert a separator
      if (lastIncluded >= 0 && si - lastIncluded > 1) {
        result.push({ type: 'separator', text: '' });
      }
      result.push(stack[si]);
      lastIncluded = si;
    }
  }

  return result;
}

/** Format arguments for display based on tool name. */
function formatArguments(name: string, raw: string): string {
  if (!raw) return '';
  const obj = tryParseJson(raw);
  if (!obj) return raw;

  // ShellExec: show only the command
  if (name === 'ShellExec' && typeof obj.command === 'string') {
    return obj.command;
  }

  // Default: pretty-print JSON
  return JSON.stringify(obj, null, 2);
}

interface FormattedResult {
  parts: Array<{ text: string; isStderr: boolean }>;
}

/** Format result for display based on tool name. */
function formatResult(name: string, raw: string): FormattedResult | null {
  if (!raw) return null;
  const obj = tryParseJson(raw);

  // ShellExec: show stderr (red) + stdout, only if non-empty
  if (obj && name === 'ShellExec') {
    const parts: FormattedResult['parts'] = [];
    const stderr = typeof obj.stderr === 'string' ? obj.stderr : '';
    const stdout = typeof obj.stdout === 'string' ? obj.stdout : '';

    if (stderr) parts.push({ text: stderr, isStderr: true });
    if (stdout) parts.push({ text: stdout, isStderr: false });

    if (parts.length > 0) return { parts };
    // If both empty, fall through to raw display
  }

  // Default: show raw result
  return { parts: [{ text: raw, isStderr: false }] };
}

/** Format ms as human-readable duration. */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  return s < 60 ? `${s.toFixed(1)}s` : `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
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
// ToolCallCard
// ---------------------------------------------------------------------------

const ACCENT_COLOR = '#00a6ffff';

export function ToolCallCard({ toolCall, status = 'success', result, durationMs, urlMeta: urlMetaProp }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);

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

  const hasExpandable = displayArgs || displayResult;

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
            {displayArgs && (
              <div className="tool-call-section">
                <div className="tool-call-label">Arguments</div>
                <pre className="tool-call-code">{displayArgs}</pre>
              </div>
            )}
            {displayResult && (
              <div className="tool-call-section">
                <div className="tool-call-label">Result</div>
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
          </div>
        )}
      </div>
    );
  }

  // ---- ShellExec inline tag rendering ----
  const shellCommand = toolCall.name === 'ShellExec' ? extractShellCommand(toolCall.arguments) : null;
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
            {displayArgs && (
              <div className="tool-call-section">
                <div className="tool-call-label">Command</div>
                <pre className="tool-call-code">{displayArgs}</pre>
              </div>
            )}
            {displayResult && (
              <div className="tool-call-section">
                <div className="tool-call-label">Output</div>
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
          </div>
        )}
      </div>
    );
  }

  // ---- ToolSearch compact tag rendering ----
  const searchMeta = toolCall.name === 'ToolSearch' ? extractToolSearchMeta(toolCall.arguments) : null;
  if (searchMeta) {
    const searchResult = useMemo(
      () => (result ? formatToolSearchResult(result) : null),
      [result],
    );
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
            ) : displayResult ? (
              <div className="tool-call-section">
                <div className="tool-call-label">Result</div>
                <pre className="tool-call-code">
                  {displayResult.parts.map((part, i) => (
                    <span key={i} className={part.isStderr ? 'tool-result-stderr' : ''}>
                      {part.text}
                      {i < displayResult.parts.length - 1 ? '\n' : ''}
                    </span>
                  ))}
                </pre>
              </div>
            ) : null}
          </div>
        )}
      </div>
    );
  }

  // ---- Glob compact tag rendering ----
  const globMeta = extractGlobMeta(toolCall.name, toolCall.arguments);
  if (globMeta) {
    const globResult = useMemo(
      () => (result ? parseGlobResult(result) : null),
      [result],
    );
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
            ) : displayResult ? (
              <div className="tool-call-section">
                <div className="tool-call-label">Result</div>
                <pre className="tool-call-code">
                  {displayResult.parts.map((part, i) => (
                    <span key={i} className={part.isStderr ? 'tool-result-stderr' : ''}>
                      {part.text}
                      {i < displayResult.parts.length - 1 ? '\n' : ''}
                    </span>
                  ))}
                </pre>
              </div>
            ) : null}
          </div>
        )}
      </div>
    );
  }

  // ---- FileEdit / FileWrite inline tag rendering ----
  const fileMeta = extractFileToolMeta(toolCall.name, toolCall.arguments);
  if (fileMeta) {
    const fileName = basename(fileMeta.filePath);
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
    const FileIcon = FILE_ICONS[fileMeta.toolType];
    const fileLabel = FILE_LABELS[fileMeta.toolType];
    const hasDiff = fileMeta.toolType === 'edit' && fileMeta.oldString !== undefined && fileMeta.newString !== undefined;
    const canExpand = hasDiff || hasExpandable;

    return (
      <div className={`tool-call-file-wrapper ${statusClass}`}>
        <div
          className="tool-call-file-tag"
          onClick={() => canExpand && setExpanded(!expanded)}
          title={fileMeta.filePath}
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
            {hasDiff && <FileDiffView oldString={fileMeta.oldString!} newString={fileMeta.newString!} />}
            {!hasDiff && displayResult && (
              <div className="tool-call-section">
                <div className="tool-call-label">Result</div>
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
      {displayArgs && (
        <div className="tool-call-section">
          <div className="tool-call-label">Arguments</div>
          <pre className="tool-call-code">{displayArgs}</pre>
        </div>
      )}
      {displayResult && (
        <div className="tool-call-section">
          <div className="tool-call-label">Result</div>
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
    </CollapsibleCard>
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
