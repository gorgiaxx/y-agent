/**
 * toolCallUtils.ts -- Shared parsing & formatting utilities for ToolCallCard.
 *
 * Extracted from ToolCallCard.tsx to keep the component focused on rendering.
 */

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

/** Try to parse JSON; return null on failure. */
export function tryParseJson(raw: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === 'object' && parsed !== null ? parsed : null;
  } catch {
    return null;
  }
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object' ? value as Record<string, unknown> : null;
}

/** Extract hostname from a URL, returning the raw string on failure. */
export function extractDomain(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

/** Extract the basename from a file path. */
export function basename(filePath: string): string {
  const parts = filePath.replace(/\\/g, '/').split('/');
  return parts[parts.length - 1] || filePath;
}

/** Map file extension to Prism language identifier for syntax highlighting. */
const EXT_TO_LANG: Record<string, string> = {
  rs: 'rust', py: 'python', js: 'javascript', jsx: 'jsx',
  ts: 'typescript', tsx: 'tsx', rb: 'ruby', go: 'go',
  java: 'java', kt: 'kotlin', kts: 'kotlin', swift: 'swift',
  c: 'c', h: 'c', cpp: 'cpp', cxx: 'cpp', cc: 'cpp', hpp: 'cpp',
  cs: 'csharp', php: 'php', lua: 'lua', r: 'r',
  sh: 'bash', bash: 'bash', zsh: 'bash', fish: 'bash',
  ps1: 'powershell', bat: 'batch',
  html: 'html', htm: 'html', css: 'css', scss: 'scss',
  sass: 'sass', less: 'less', xml: 'xml', svg: 'xml',
  json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml',
  ini: 'ini', cfg: 'ini',
  md: 'markdown', mdx: 'markdown',
  sql: 'sql', graphql: 'graphql', gql: 'graphql',
  dockerfile: 'docker', proto: 'protobuf',
  makefile: 'makefile', cmake: 'cmake',
  zig: 'zig', nim: 'nim', dart: 'dart', scala: 'scala',
  ex: 'elixir', exs: 'elixir', erl: 'erlang',
  hs: 'haskell', ml: 'ocaml', clj: 'clojure',
  tf: 'hcl', hcl: 'hcl',
};

/** Infer Prism language from a file path based on its extension. */
export function inferLanguage(filePath: string): string {
  const name = basename(filePath).toLowerCase();
  // Handle extensionless filenames like Dockerfile, Makefile
  if (name === 'dockerfile') return 'docker';
  if (name === 'makefile' || name === 'gnumakefile') return 'makefile';
  if (name === 'cmakelists.txt') return 'cmake';
  const dot = name.lastIndexOf('.');
  if (dot < 0) return 'text';
  const ext = name.slice(dot + 1);
  return EXT_TO_LANG[ext] ?? 'text';
}

// ---------------------------------------------------------------------------
// URL metadata extraction for Browser/WebFetch tool calls
// ---------------------------------------------------------------------------

export interface UrlToolMeta {
  url: string;
  title: string;
  faviconUrl: string;
  domain: string;
}

export interface BrowserActionSummary {
  action: string;
  label: string;
  detail?: string;
}

const KNOWN_TOOL_NAMES = [
  'AskUser',
  'Browser',
  'FileEdit',
  'FileRead',
  'FileWrite',
  'Glob',
  'Grep',
  'KnowledgeSearch',
  'Plan',
  'PlanWriter',
  'ShellExec',
  'ToolSearch',
  'WebFetch',
];

const BROWSER_INPUT_FIELDS = new Set([
  'action',
  'url',
  'query',
  'search_engine',
  'wait_ms',
  'selector',
  'text',
  'expression',
  'full_page',
  'format',
  'interactive_only',
  'key',
  'direction',
  'pixels',
  'ms',
  'limit',
  'max_text_chars',
  'quality',
]);

const WEBFETCH_INPUT_FIELDS = new Set([
  'action',
  'url',
  'query',
  'search_engine',
  'wait_ms',
]);

function pickFields(
  source: Record<string, unknown>,
  allowedFields: Set<string>,
): Record<string, unknown> {
  const filtered: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(source)) {
    if (allowedFields.has(key)) {
      filtered[key] = value;
    }
  }

  return filtered;
}

function sanitizeToolArguments(
  toolName: string,
  source: Record<string, unknown>,
): Record<string, unknown> {
  if (toolName === 'Browser') {
    return pickFields(source, BROWSER_INPUT_FIELDS);
  }
  if (toolName === 'WebFetch') {
    return pickFields(source, WEBFETCH_INPUT_FIELDS);
  }
  return source;
}

export function canonicalToolName(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return trimmed;

  const lowered = trimmed.toLowerCase();
  const canonical = KNOWN_TOOL_NAMES.find((candidate) => candidate.toLowerCase() === lowered);
  return canonical ?? trimmed;
}

function extractLooseStringField(raw: string | undefined, key: string): string | undefined {
  if (!raw) return undefined;

  const patterns = [
    new RegExp(`"${key}"\\s*:\\s*"([^"]+)"`),
    new RegExp(`<${key}>\\s*([^<]+?)\\s*</${key}>`, 'i'),
    new RegExp(`<parameter=${key}>\\s*([^<]+?)\\s*</parameter>`, 'i'),
  ];

  for (const pattern of patterns) {
    const match = raw.match(pattern);
    const value = match?.[1]?.trim();
    if (value) return value;
  }

  return undefined;
}

function summarizeBrowserText(value: unknown, maxLength = 48): string | undefined {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (trimmed.length <= maxLength) return trimmed;
  return `${trimmed.slice(0, maxLength - 1)}…`;
}

function titleCaseAction(action: string): string {
  return action
    .split('_')
    .filter(Boolean)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join(' ');
}

function valueEquals(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function extractBrowserActionSummary(
  argsRaw: string,
  resultRaw?: string,
): BrowserActionSummary | null {
  const args = tryParseJson(argsRaw);
  const result = resultRaw ? tryParseJson(resultRaw) : null;
  const source = args ?? result;
  const action = typeof source?.action === 'string'
    ? source.action
    : (
      extractLooseStringField(argsRaw, 'action')
      ?? extractLooseStringField(resultRaw, 'action')
      ?? ''
    );

  if (!action) return null;

  let detail: string | undefined;

  switch (action) {
    case 'click':
    case 'getText':
    case 'type':
      detail = summarizeBrowserText(source?.selector)
        ?? extractLooseStringField(argsRaw, 'selector');
      break;
    case 'pressKey':
      detail = summarizeBrowserText(source?.key)
        ?? extractLooseStringField(argsRaw, 'key');
      break;
    case 'scroll': {
      const direction = summarizeBrowserText(source?.direction)
        ?? extractLooseStringField(argsRaw, 'direction')
        ?? 'down';
      const pixels = typeof source?.pixels === 'number' ? `${source.pixels}px` : undefined;
      detail = pixels ? `${direction} ${pixels}` : direction;
      break;
    }
    case 'wait':
      detail = summarizeBrowserText(source?.selector)
        ?? extractLooseStringField(argsRaw, 'selector')
        ?? (typeof source?.ms === 'number' ? `${source.ms} ms` : undefined);
      break;
    case 'snapshot':
      detail = summarizeBrowserText(source?.format)
        ?? extractLooseStringField(argsRaw, 'format')
        ?? (source?.interactive_only === true ? 'interactive' : undefined);
      break;
    case 'screenshot':
      detail = source?.full_page === true ? 'full page' : 'viewport';
      break;
    case 'evaluate':
      detail = summarizeBrowserText(source?.expression)
        ?? extractLooseStringField(argsRaw, 'expression');
      break;
    case 'search':
      detail = summarizeBrowserText(source?.query)
        ?? extractLooseStringField(argsRaw, 'query');
      break;
    case 'navigate':
      detail = summarizeBrowserText(source?.url)
        ?? extractLooseStringField(argsRaw, 'url');
      break;
    default:
      detail = summarizeBrowserText(source?.selector)
        ?? extractLooseStringField(argsRaw, 'selector')
        ?? summarizeBrowserText(source?.url)
        ?? extractLooseStringField(argsRaw, 'url')
        ?? summarizeBrowserText(source?.query)
        ?? extractLooseStringField(argsRaw, 'query');
      break;
  }

  return {
    action,
    label: titleCaseAction(action),
    detail,
  };
}

/** Extract URL metadata from Browser/WebFetch tool calls. */
export function extractUrlMeta(
  toolName: string,
  argsRaw: string,
  resultRaw?: string,
): UrlToolMeta | null {
  toolName = canonicalToolName(toolName);
  const parsedArgs = tryParseJson(argsRaw);
  const parsedResult = resultRaw ? tryParseJson(resultRaw) : null;

  if (toolName === 'Browser') {
    // Detect navigate/search from arguments or from result action field
    const action = String(
      parsedArgs?.action
      ?? parsedResult?.action
      ?? extractLooseStringField(argsRaw, 'action')
      ?? extractLooseStringField(resultRaw, 'action')
      ?? '',
    );
    if (action === 'navigate' || action === 'search') {
      const url = String(
        parsedResult?.url
        ?? parsedArgs?.url
        ?? parsedArgs?.query
        ?? extractLooseStringField(argsRaw, 'url')
        ?? extractLooseStringField(argsRaw, 'query')
        ?? '',
      );
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
    const url = String(
      parsedResult?.url
      ?? parsedArgs?.url
      ?? parsedArgs?.query
      ?? extractLooseStringField(argsRaw, 'url')
      ?? extractLooseStringField(argsRaw, 'query')
      ?? '',
    );
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

// ---------------------------------------------------------------------------
// ShellExec helpers
// ---------------------------------------------------------------------------

/** Extract the command string from ShellExec arguments. */
export function extractShellCommand(argsRaw: string): string | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  if (typeof obj.command === 'string') return obj.command;
  return null;
}

// ---------------------------------------------------------------------------
// ToolSearch helpers
// ---------------------------------------------------------------------------

export interface ToolSearchMeta {
  /** The search mode key: 'query', 'category', or 'tool'. */
  key: string;
  /** The search value. */
  value: string;
}

/** Extract the search key:value from ToolSearch arguments. */
export function extractToolSearchMeta(argsRaw: string): ToolSearchMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  // Precedence: tool > category > query (matches backend).
  if (typeof obj.tool === 'string' && obj.tool) return { key: 'tool', value: obj.tool };
  if (typeof obj.category === 'string' && obj.category) return { key: 'category', value: obj.category };
  if (typeof obj.query === 'string' && obj.query) return { key: 'query', value: obj.query };
  return null;
}

/** Format ToolSearch result for structured display. */
export function formatToolSearchResult(raw: string): { lines: Array<{ label: string; items: string[] }> } | null {
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
// Grep helpers
// ---------------------------------------------------------------------------

export interface GrepMeta {
  pattern: string;
  path?: string;
  outputMode?: 'files_with_matches' | 'content' | 'count';
  glob?: string;
  type?: string;
  caseInsensitive?: boolean;
}

export interface GrepResult {
  mode: 'files_with_matches' | 'content' | 'count';
  numFiles: number;
  numLines?: number;
  numMatches?: number;
  filenames?: string[];
  content?: string;
  appliedLimit?: number;
  truncated?: boolean;
}

/** Extract grep metadata from Grep tool call arguments. */
export function extractGrepMeta(toolName: string, argsRaw: string): GrepMeta | null {
  if (toolName !== 'Grep') return null;
  try {
    const args = JSON.parse(argsRaw);
    return {
      pattern: args.pattern || '',
      path: args.path,
      outputMode: args.output_mode,
      glob: args.Glob,
      type: args.type,
      caseInsensitive: args.i,
    };
  } catch {
    return null;
  }
}

/** Parse the structured Grep result JSON. */
export function parseGrepResult(raw: string): GrepResult | null {
  try {
    const data = JSON.parse(raw);
    // Determine mode based on which fields are present
    if (data.filenames !== undefined) {
      return {
        mode: 'files_with_matches',
        numFiles: data.numFiles ?? 0,
        filenames: data.filenames ?? [],
        appliedLimit: data.appliedLimit,
        truncated: data.appliedLimit && data.numFiles >= data.appliedLimit,
      };
    } else if (data.numMatches !== undefined) {
      return {
        mode: 'count',
        numFiles: data.numFiles ?? 0,
        numMatches: data.numMatches ?? 0,
        content: data.content,
        appliedLimit: data.appliedLimit,
      };
    } else if (data.numLines !== undefined) {
      return {
        mode: 'content',
        numFiles: data.numFiles ?? 0,
        numLines: data.numLines ?? 0,
        content: data.content,
        appliedLimit: data.appliedLimit,
      };
    }
    return null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Glob helpers
// ---------------------------------------------------------------------------

export interface GlobMeta {
  pattern: string;
  searchPath?: string;
}

export interface GlobResult {
  matches: string[];
  count: number;
  truncated: boolean;
}

/** Extract glob metadata from Glob tool call arguments. */
export function extractGlobMeta(toolName: string, argsRaw: string): GlobMeta | null {
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

/** Parse the structured Glob result JSON. */
export function parseGlobResult(raw: string): GlobResult | null {
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

export type FileToolType = 'read' | 'edit' | 'write';

export interface FileToolMeta {
  filePath: string;
  toolType: FileToolType;
  oldString?: string;
  newString?: string;
}

/** Extract file metadata from FileRead/FileEdit/FileWrite tool call arguments. */
export function extractFileToolMeta(
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

// ---------------------------------------------------------------------------
// Diff computation
// ---------------------------------------------------------------------------

export interface DiffLine {
  type: 'context' | 'add' | 'remove' | 'separator';
  text: string;
  /** Line number in the old file (1-based). Undefined for additions/separators. */
  oldLineNo?: number;
  /** Line number in the new file (1-based). Undefined for removals/separators. */
  newLineNo?: number;
}

export function computeLineDiff(oldStr: string, newStr: string): DiffLine[] {
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

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

export interface FormattedResult {
  parts: Array<{ text: string; isStderr: boolean }>;
}

/** Strip noisy internal fields from Browser/WebFetch result JSON. */
function stripUrlMetaFields(raw: string): string {
  const obj = tryParseJson(raw);
  if (!obj) return raw;
  // Remove favicon_url -- already consumed by the Favicon component.
  const { favicon_url: _faviconUrl, ...rest } = obj as Record<string, unknown>;
  void _faviconUrl;
  return JSON.stringify(rest, null, 2);
}

/** Format arguments for display based on tool name. */
export function formatArguments(name: string, raw: string): string {
  name = canonicalToolName(name);
  if (!raw) return '';
  const obj = tryParseJson(raw);
  if (!obj) return raw;

  // ShellExec: show only the command
  if (name === 'ShellExec' && typeof obj.command === 'string') {
    return obj.command;
  }

  if (name === 'Browser' || name === 'WebFetch') {
    return JSON.stringify(sanitizeToolArguments(name, obj), null, 2);
  }

  // Default: pretty-print JSON
  return JSON.stringify(obj, null, 2);
}

/** Format result for display based on tool name (raw mode -- shows all fields). */
export function formatResult(name: string, raw: string): FormattedResult | null {
  name = canonicalToolName(name);
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

  // Browser/WebFetch: strip favicon_url before display
  if (name === 'Browser' || name === 'WebFetch') {
    return { parts: [{ text: stripUrlMetaFields(raw), isStderr: false }] };
  }

  // Default: show raw result
  return { parts: [{ text: raw, isStderr: false }] };
}

/**
 * Format result for formatted (non-raw) display.
 * Extracts the most meaningful field per tool type:
 * - ShellExec: stdout only
 * - FileRead: content field only
 * - FileWrite: content field from arguments (result has bytes_written)
 * - FileEdit: content field only
 * - Others: delegates to formatResult
 */
export function formatResultFormatted(
  name: string,
  raw: string,
  argsRaw?: string,
): FormattedResult | null {
  name = canonicalToolName(name);
  if (!raw) return null;
  const obj = tryParseJson(raw);

  // ShellExec: show only stdout
  if (obj && name === 'ShellExec') {
    const stdout = typeof obj.stdout === 'string' ? obj.stdout : '';
    if (stdout) return { parts: [{ text: stdout, isStderr: false }] };
    // If stdout empty but stderr present, show nothing in formatted mode
    // (stderr is noise in formatted view)
    return null;
  }

  // FileRead: show only the content field
  if (obj && name === 'FileRead') {
    const content = typeof obj.content === 'string' ? obj.content : '';
    if (content) return { parts: [{ text: content, isStderr: false }] };
    return null;
  }

  // FileWrite: show the content from arguments (result only has bytes_written)
  if (name === 'FileWrite' && argsRaw) {
    const argsObj = tryParseJson(argsRaw);
    if (argsObj) {
      const content = typeof argsObj.content === 'string' ? argsObj.content : '';
      if (content) return { parts: [{ text: content, isStderr: false }] };
    }
    return null;
  }

  // FileEdit: show the content field if present in result
  if (obj && name === 'FileEdit') {
    const content = typeof obj.content === 'string' ? obj.content : '';
    if (content) return { parts: [{ text: content, isStderr: false }] };
    // Fall through to default
  }

  if (obj && name === 'Browser') {
    const args = argsRaw ? tryParseJson(argsRaw) : null;
    const meaningful: Record<string, unknown> = { ...obj };

    delete meaningful.action;

    if (args) {
      const sanitizedArgs = sanitizeToolArguments(name, args);
      for (const [key, value] of Object.entries(sanitizedArgs)) {
        if (key in meaningful && valueEquals(meaningful[key], value)) {
          delete meaningful[key];
        }
      }
    }

    if (typeof meaningful.text === 'string' && meaningful.text.trim()) {
      return { parts: [{ text: meaningful.text, isStderr: false }] };
    }

    if (
      Object.keys(meaningful).length === 1
      && meaningful.ok === true
    ) {
      return { parts: [{ text: 'Success', isStderr: false }] };
    }

    if (Object.keys(meaningful).length > 0) {
      return {
        parts: [{ text: JSON.stringify(meaningful, null, 2), isStderr: false }],
      };
    }
  }

  // Others: use default formatting
  return formatResult(name, raw);
}

// ---------------------------------------------------------------------------
// AskUser helpers
// ---------------------------------------------------------------------------

export interface AskUserQuestion {
  question: string;
  options: string[];
  multi_select?: boolean;
}

export interface AskUserMeta {
  questions: AskUserQuestion[];
  status: string;
}

export interface AskUserResult {
  answers: Record<string, string>;
  status: string;
}

/** Extract AskUser metadata from the tool call arguments or result JSON. */
export function extractAskUserMeta(argsRaw: string, resultRaw?: string): AskUserMeta | null {
  // Prefer result metadata when it includes the questions, otherwise fall back
  // to the original tool arguments so answered AskUser cards still retain
  // their question structure in streaming mode.
  const resultSource = resultRaw ? tryParseJson(resultRaw) : null;
  const resultQuestions = resultSource?.questions;
  const argsSource = tryParseJson(argsRaw);
  const argsQuestions = argsSource?.questions;
  const questions = Array.isArray(resultQuestions) && resultQuestions.length > 0
    ? resultQuestions
    : argsQuestions;

  if (!Array.isArray(questions) || questions.length === 0) return null;

  return {
    questions: questions as AskUserQuestion[],
    status: typeof resultSource?.status === 'string'
      ? resultSource.status
      : (resultSource?.answers && typeof resultSource.answers === 'object' ? 'answered' : 'pending'),
  };
}

/** Parse the AskUser result to extract final answers. */
export function parseAskUserResult(raw: string): AskUserResult | null {
  const obj = tryParseJson(raw);
  if (!obj) return null;
  const answers = obj.answers;
  if (!answers || typeof answers !== 'object') return null;
  return {
    answers: answers as Record<string, string>,
    status: typeof obj.status === 'string' ? obj.status : 'unknown',
  };
}

// ---------------------------------------------------------------------------
// PlanWriter helpers
// ---------------------------------------------------------------------------

export interface PlanWriterMeta {
  title: string;
  content: string;
}

export interface PlanWriterResult {
  path: string;
  title: string;
}

/** Extract PlanWriter metadata from tool call arguments. */
export function extractPlanWriterMeta(argsRaw: string): PlanWriterMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const title = typeof obj.title === 'string' ? obj.title : '';
  const content = typeof obj.content === 'string' ? obj.content : '';
  if (!title) return null;
  return { title, content };
}

/** Parse PlanWriter result to extract the written file path. */
export function parsePlanWriterResult(raw: string): PlanWriterResult | null {
  const obj = tryParseJson(raw);
  if (!obj) return null;
  const path = typeof obj.path === 'string' ? obj.path : '';
  const title = typeof obj.title === 'string' ? obj.title : '';
  if (!path) return null;
  return { path, title };
}

export interface PlanRequestMeta {
  request: string;
  context: string;
}

export interface PlanTaskDisplay {
  id: string;
  phase: number;
  title: string;
  description: string;
  dependsOn: string[];
  status: string;
  estimatedIterations: number;
  keyFiles: string[];
  acceptanceCriteria: string[];
}

export interface PlanWriterStageDisplay {
  kind: 'plan_stage';
  stage: 'plan_writer';
  stageStatus: string;
  planTitle: string;
  planFile: string;
  planContent: string;
}

export interface TaskDecomposerStageDisplay {
  kind: 'plan_stage';
  stage: 'task_decomposer';
  stageStatus: string;
  planTitle: string;
  planFile: string;
  tasks: PlanTaskDisplay[];
}

export interface PlanExecutionDisplay {
  kind: 'plan_execution';
  planTitle: string;
  planFile: string;
  totalPhases: number;
  completed: number;
  failed: number;
  tasks: PlanTaskDisplay[];
  phases: Array<Record<string, unknown>>;
}

export type PlanDisplayMeta =
  | PlanWriterStageDisplay
  | TaskDecomposerStageDisplay
  | PlanExecutionDisplay;

export function extractPlanRequestMeta(argsRaw: string): PlanRequestMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const request = typeof obj.request === 'string' ? obj.request : '';
  if (!request) return null;
  return {
    request,
    context: typeof obj.context === 'string' ? obj.context : '',
  };
}

function parsePlanTask(value: unknown): PlanTaskDisplay | null {
  const obj = asObject(value);
  if (!obj) return null;
  const title = typeof obj.title === 'string' ? obj.title : '';
  if (!title) return null;
  return {
    id: typeof obj.id === 'string' ? obj.id : '',
    phase: typeof obj.phase === 'number' ? obj.phase : 0,
    title,
    description: typeof obj.description === 'string' ? obj.description : '',
    dependsOn: Array.isArray(obj.depends_on)
      ? obj.depends_on.map((dep) => String(dep))
      : [],
    status: typeof obj.status === 'string' ? obj.status : 'pending',
    estimatedIterations: typeof obj.estimated_iterations === 'number'
      ? obj.estimated_iterations
      : 0,
    keyFiles: Array.isArray(obj.key_files)
      ? obj.key_files.map((file) => String(file))
      : [],
    acceptanceCriteria: Array.isArray(obj.acceptance_criteria)
      ? obj.acceptance_criteria.map((item) => String(item))
      : [],
  };
}

function mergeExecutionTaskStatuses(
  tasks: PlanTaskDisplay[],
  phases: Array<Record<string, unknown>>,
): PlanTaskDisplay[] {
  if (tasks.length === 0 || phases.length === 0) return tasks;

  const statusByTaskId = new Map<string, string>();
  const statusByPhase = new Map<number, string>();
  const statusByTitle = new Map<string, string>();

  for (const phase of phases) {
    const status = typeof phase.status === 'string' ? phase.status : '';
    if (!status) continue;

    if (typeof phase.task_id === 'string' && phase.task_id) {
      statusByTaskId.set(phase.task_id, status);
    }
    if (typeof phase.phase === 'number') {
      statusByPhase.set(phase.phase, status);
    }
    if (typeof phase.title === 'string' && phase.title) {
      statusByTitle.set(phase.title, status);
    }
  }

  return tasks.map((task) => ({
    ...task,
    status: statusByTaskId.get(task.id)
      ?? statusByPhase.get(task.phase)
      ?? statusByTitle.get(task.title)
      ?? task.status,
  }));
}

function parsePlanDisplayObject(obj: Record<string, unknown>): PlanDisplayMeta | null {
  const kind = typeof obj.kind === 'string' ? obj.kind : '';

  if (kind === 'plan_stage') {
    const stage = typeof obj.stage === 'string' ? obj.stage : '';
    const stageStatus = typeof obj.stage_status === 'string' ? obj.stage_status : 'completed';
    const planTitle = typeof obj.plan_title === 'string' ? obj.plan_title : '';
    const planFile = typeof obj.plan_file === 'string' ? obj.plan_file : '';

    if (stage === 'plan_writer') {
      return {
        kind: 'plan_stage',
        stage,
        stageStatus,
        planTitle,
        planFile,
        planContent: typeof obj.plan_content === 'string' ? obj.plan_content : '',
      };
    }

    if (stage === 'task_decomposer') {
      const tasks = Array.isArray(obj.tasks)
        ? obj.tasks.map(parsePlanTask).filter((task): task is PlanTaskDisplay => task != null)
        : [];
      return {
        kind: 'plan_stage',
        stage,
        stageStatus,
        planTitle,
        planFile,
        tasks,
      };
    }
  }

  if (kind === 'plan_execution') {
    const hasPlanFields = typeof obj.plan_title === 'string'
      || typeof obj.plan_file === 'string'
      || typeof obj.total_phases === 'number';
    if (!hasPlanFields) return null;

    const tasks = Array.isArray(obj.tasks)
      ? obj.tasks.map(parsePlanTask).filter((task): task is PlanTaskDisplay => task != null)
      : [];
    const phases = Array.isArray(obj.phases)
      ? obj.phases.filter((phase): phase is Record<string, unknown> => phase != null && typeof phase === 'object')
      : [];
    const mergedTasks = mergeExecutionTaskStatuses(tasks, phases);

    return {
      kind: 'plan_execution',
      planTitle: typeof obj.plan_title === 'string' ? obj.plan_title : '',
      planFile: typeof obj.plan_file === 'string' ? obj.plan_file : '',
      totalPhases: typeof obj.total_phases === 'number' ? obj.total_phases : tasks.length,
      completed: typeof obj.completed === 'number' ? obj.completed : 0,
      failed: typeof obj.failed === 'number' ? obj.failed : 0,
      tasks: mergedTasks,
      phases,
    };
  }

  return null;
}

export function extractPlanDisplayMeta(
  metadata: unknown,
  resultRaw?: string,
): PlanDisplayMeta | null {
  const metaObj = asObject(metadata);
  const displayObj = asObject(metaObj?.display);
  if (displayObj) {
    const display = parsePlanDisplayObject(displayObj);
    if (display) return display;
  }

  const resultObj = resultRaw ? tryParseJson(resultRaw) : null;
  if (resultObj) {
    return parsePlanDisplayObject({
      kind: 'plan_execution',
      ...resultObj,
    });
  }

  return null;
}

// ---------------------------------------------------------------------------
// KnowledgeSearch helpers
// ---------------------------------------------------------------------------

export interface KnowledgeSearchMeta {
  query: string;
  domain?: string;
  limit?: number;
}

export interface KnowledgeSearchResultItem {
  title: string;
  relevance: string;
  chunkId: string;
}

export interface KnowledgeSearchResult {
  count: number;
  results: KnowledgeSearchResultItem[];
  truncated: boolean;
}

export function extractKnowledgeSearchMeta(argsRaw: string): KnowledgeSearchMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const query = typeof obj.query === 'string' ? obj.query : '';
  if (!query) return null;
  return {
    query,
    domain: typeof obj.domain === 'string' ? obj.domain : undefined,
    limit: typeof obj.limit === 'number' ? obj.limit : undefined,
  };
}

export function parseKnowledgeSearchResult(raw: string): KnowledgeSearchResult | null {
  const obj = tryParseJson(raw);
  if (!obj) return null;
  if (!Array.isArray(obj.results)) return null;
  const results = (obj.results as Array<Record<string, unknown>>).map((r) => ({
    title: typeof r.title === 'string' ? r.title : '',
    relevance: typeof r.relevance === 'string' ? r.relevance : String(r.relevance ?? ''),
    chunkId: typeof r.chunk_id === 'string' ? r.chunk_id : '',
  }));
  return {
    count: typeof obj.count === 'number' ? obj.count : results.length,
    results,
    truncated: obj.truncated === true,
  };
}
