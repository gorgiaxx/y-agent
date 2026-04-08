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

// ---------------------------------------------------------------------------
// URL metadata extraction for Browser/WebFetch tool calls
// ---------------------------------------------------------------------------

export interface UrlToolMeta {
  url: string;
  title: string;
  faviconUrl: string;
  domain: string;
}

/** Extract URL metadata from Browser/WebFetch tool calls. */
export function extractUrlMeta(
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

/** Format result for display based on tool name (raw mode -- shows all fields). */
export function formatResult(name: string, raw: string): FormattedResult | null {
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
  // Try result first (it contains the full payload), then fall back to args.
  const source = resultRaw ? tryParseJson(resultRaw) : tryParseJson(argsRaw);
  if (!source) return null;

  const questions = source.questions;
  if (!Array.isArray(questions) || questions.length === 0) return null;

  return {
    questions: questions as AskUserQuestion[],
    status: typeof source.status === 'string' ? source.status : 'pending',
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
// PlanWriter / ExitPlanMode helpers
// ---------------------------------------------------------------------------

export interface PlanWriterMeta {
  title: string;
  content: string;
}

export interface PlanWriterResult {
  path: string;
  title: string;
}

export interface ExitPlanModeMeta {
  planFile: string;
  totalPhases: number;
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

/** Extract ExitPlanMode metadata from tool call arguments. */
export function extractExitPlanModeMeta(argsRaw: string): ExitPlanModeMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const planFile = typeof obj.plan_file === 'string' ? obj.plan_file : '';
  const totalPhases = typeof obj.total_phases === 'number' ? obj.total_phases : 0;
  if (!planFile) return null;
  return { planFile, totalPhases };
}
