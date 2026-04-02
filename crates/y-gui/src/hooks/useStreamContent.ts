// Stream content processor for lazy output rendering.
//
// Parses `<tool_call>...</tool_call>` XML blocks from streaming LLM text
// into structured data. Text segments are returned for markdown rendering,
// tool call blocks are returned as structured data for card rendering.

const TOOL_CALL_TAG = 'tool_call';
const TOOL_CALL_OPEN = `<${TOOL_CALL_TAG}`;
const TOOL_CALL_CLOSE = `</${TOOL_CALL_TAG}>`;

// tool_result tags emitted by the backend (or hallucinated by the LLM)
// must be stripped from display content.
const TOOL_RESULT_OPEN = '<tool_result';
const TOOL_RESULT_CLOSE = '</tool_result>';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A parsed tool call extracted from the streaming content. */
export interface ParsedToolCall {
  /** Tool name (from <name> tag). */
  name: string;
  /** Raw arguments JSON (from <arguments> tag). */
  arguments: string;
  /** Character index in the original content where this block starts. */
  startIndex: number;
}

/** A segment of content: either plain text or a tool call. */
export type ContentSegment =
  | { type: 'text'; text: string }
  | { type: 'tool_call'; toolCall: ParsedToolCall };

export interface StreamContentResult {
  /** Ordered segments: text interspersed with tool calls. */
  segments: ContentSegment[];
  /** Plain text only (all tool_call blocks removed). */
  displayText: string;
  /** Parsed tool calls found in the content. */
  toolCalls: ParsedToolCall[];
  /** True if there is buffered content waiting for a closing tag. */
  hasPendingToolCall: boolean;
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/** Extract text content of an XML tag from a block, e.g. <name>foo</name> → "foo". */
function extractTag(block: string, tagName: string): string {
  block = block.trim();
  const open = `<${tagName}>`;
  const close = `</${tagName}>`;
  const start = block.indexOf(open);
  if (start < 0) return '';
  const contentStart = start + open.length;
  const end = block.indexOf(close, contentStart);
  if (end < 0) return block.slice(contentStart).trim();
  return block.slice(contentStart, end).trim();
}

/**
 * Try to parse a tool_call block using function-attribute format.
 *
 * Handles Llama/Qwen-family models that emit:
 *   <tool_call>
 *   <function=browser>
 *   <parameter=url>https://example.com</parameter>
 *   </function>
 *   </tool_call>
 */
function tryParseFunctionFormat(block: string): { name: string; arguments: string } | null {
  // Extract inner content between <tool_call...> and </tool_call>.
  const openEnd = block.indexOf('>');
  if (openEnd < 0) return null;
  const closeStart = block.indexOf('</tool_call>');
  if (closeStart < 0) return null;
  const inner = block.slice(openEnd + 1, closeStart).trim();
  if (!inner) return null;

  // Match <function=NAME>
  const funcMatch = inner.match(/<function=([^>]+)>/);
  if (!funcMatch) return null;
  const name = funcMatch[1].trim();
  if (!name) return null;

  // Extract body inside <function=NAME>...</function>
  const funcOpenEnd = inner.indexOf('>', inner.indexOf('<function=')) + 1;
  const funcCloseIdx = inner.indexOf('</function>');
  const body = funcCloseIdx >= 0
    ? inner.slice(funcOpenEnd, funcCloseIdx).trim()
    : inner.slice(funcOpenEnd).trim();

  // Collect <parameter=KEY>VALUE</parameter> entries.
  const args: Record<string, string> = {};
  const paramRegex = /<parameter=([^>]+)>([\s\S]*?)<\/parameter>/g;
  let paramMatch;
  while ((paramMatch = paramRegex.exec(body)) !== null) {
    const key = paramMatch[1].trim();
    const val = paramMatch[2].trim();
    if (key) args[key] = val;
  }

  // Also extract <action>VALUE</action>.
  const actionMatch = body.match(/<action>([\s\S]*?)<\/action>/);
  if (actionMatch) {
    args['action'] = actionMatch[1].trim();
  }

  // If body is JSON and no params found, parse it.
  if (Object.keys(args).length === 0 && body.startsWith('{')) {
    try {
      JSON.parse(body);
      return { name, arguments: body };
    } catch { /* not JSON */ }
  }

  return {
    name,
    arguments: Object.keys(args).length > 0 ? JSON.stringify(args, null, 2) : '',
  };
}

/**
 * Try to parse the inner content of a tool_call block as JSON.
 *
 * Handles the case where the LLM emits JSON format instead of XML-nested:
 *   <tool_call>{"name": "tool", "arguments": {"key": "val"}}</tool_call>
 *
 * This mirrors the dual-format parsing in the Rust backend (parser.rs).
 */
function tryParseToolCallJson(block: string): { name: string; arguments: string } | null {
  // Extract inner content between <tool_call> and </tool_call>.
  const closeTag = '</tool_call>';
  const openEnd = block.indexOf('>');
  if (openEnd < 0) return null;
  const closeStart = block.indexOf(closeTag);
  if (closeStart < 0) return null;
  const inner = block.slice(openEnd + 1, closeStart).trim();
  if (!inner) return null;

  try {
    const parsed = JSON.parse(inner);
    if (typeof parsed === 'object' && parsed !== null && typeof parsed.name === 'string') {
      const args = parsed.arguments
        ? (typeof parsed.arguments === 'string'
            ? parsed.arguments
            : JSON.stringify(parsed.arguments, null, 2))
        : '';
      return { name: parsed.name, arguments: args };
    }
  } catch {
    // Not valid JSON -- fall through.
  }
  return null;
}

// ---------------------------------------------------------------------------
// tool_result stripping
// ---------------------------------------------------------------------------

/**
 * Strip all `<tool_result ...>...</tool_result>` blocks from the input.
 *
 * These blocks are injected by the backend as context for subsequent LLM
 * iterations and may also be hallucinated by the model.  They must never
 * appear in the rendered chat content.
 *
 * Also strips any trailing incomplete `<tool_result` prefix so partial
 * XML is not shown while streaming.
 */
function stripToolResultBlocks(input: string): string {
  let result = '';
  let i = 0;

  while (i < input.length) {
    const openIdx = input.indexOf(TOOL_RESULT_OPEN, i);
    if (openIdx < 0) {
      result += input.slice(i);
      break;
    }

    // Add text before the tag.
    result += input.slice(i, openIdx);

    // Find matching close tag.
    const closeIdx = input.indexOf(TOOL_RESULT_CLOSE, openIdx);
    if (closeIdx >= 0) {
      // Complete block -- skip it entirely.
      i = closeIdx + TOOL_RESULT_CLOSE.length;
    } else {
      // Incomplete block -- strip everything from here to end (buffering).
      break;
    }
  }

  // Also strip a trailing partial `<tool_result` prefix that might be
  // streaming in character by character.
  const trailingIdx = result.lastIndexOf('<');
  if (trailingIdx >= 0) {
    const trailing = result.slice(trailingIdx);
    if (TOOL_RESULT_OPEN.startsWith(trailing) && trailing.length < TOOL_RESULT_OPEN.length) {
      result = result.slice(0, trailingIdx);
    }
  }

  return result;
}

// ---------------------------------------------------------------------------
// Main processor
// ---------------------------------------------------------------------------

/**
 * Process raw LLM content to produce display-safe segments.
 *
 * - Strips `<tool_result>` blocks (backend-injected or hallucinated).
 * - Parses complete `<tool_call>...</tool_call>` blocks into structured data.
 * - Buffers any trailing partial `<tool_call>` tag so it is not shown.
 * - Returns ordered segments (text + tool_call) for rendering.
 *
 * Pure function applied to the full accumulated content.
 */
export function processStreamContent(raw: string): StreamContentResult {
  // Pre-process: strip tool_result blocks before parsing tool_call segments.
  const cleaned = stripToolResultBlocks(raw);

  const segments: ContentSegment[] = [];
  const toolCalls: ParsedToolCall[] = [];
  let hasPendingToolCall = false;
  let textBuffer = '';
  let i = 0;

  const flushText = () => {
    if (textBuffer) {
      segments.push({ type: 'text', text: textBuffer });
      textBuffer = '';
    }
  };

  while (i < cleaned.length) {
    const openIdx = cleaned.indexOf('<', i);

    if (openIdx < 0) {
      textBuffer += cleaned.slice(i);
      break;
    }

    // Add text before the `<`.
    textBuffer += cleaned.slice(i, openIdx);

    const remaining = cleaned.slice(openIdx);

    if (remaining.startsWith(TOOL_CALL_OPEN)) {
      // Look for the matching closing tag.
      const closeIdx = cleaned.indexOf(TOOL_CALL_CLOSE, openIdx);
      if (closeIdx >= 0) {
        // Complete tool_call block — parse it.
        const blockEnd = closeIdx + TOOL_CALL_CLOSE.length;
        const block = cleaned.slice(openIdx, blockEnd);

        // Try XML-nested format first (primary), then JSON fallback.
        const xmlName = extractTag(block, 'name');
        const xmlArgs = extractTag(block, 'arguments');

        let tcName: string;
        let tcArgs: string;

        if (xmlName) {
          // XML-nested format: <name>tool</name><arguments>...</arguments>
          tcName = xmlName;
          tcArgs = xmlArgs;
        } else {
          // Try function-attribute format: <function=name><parameter=k>v</parameter></function>
          const funcResult = tryParseFunctionFormat(block);
          if (funcResult) {
            tcName = funcResult.name;
            tcArgs = funcResult.arguments;
          } else {
            // JSON fallback: {"name": "tool", "arguments": {...}}
            const jsonResult = tryParseToolCallJson(block);
            if (jsonResult) {
              tcName = jsonResult.name;
              tcArgs = jsonResult.arguments;
            } else {
              tcName = 'unknown';
              tcArgs = xmlArgs;
            }
          }
        }

        const tc: ParsedToolCall = {
          name: tcName,
          arguments: tcArgs,
          startIndex: openIdx,
        };
        toolCalls.push(tc);

        // Flush any pending text, then add the tool call segment.
        flushText();
        segments.push({ type: 'tool_call', toolCall: tc });

        i = blockEnd;
        continue;
      } else {
        // Incomplete tool_call tag — buffer it.
        hasPendingToolCall = true;
        break;
      }
    }

    // Check for orphaned closing tag.
    if (remaining.startsWith(TOOL_CALL_CLOSE)) {
      i = openIdx + TOOL_CALL_CLOSE.length;
      continue;
    }

    // Check for partial prefix match at end of buffer.
    if (isPartialToolCallPrefix(remaining) && openIdx + remaining.length === cleaned.length) {
      hasPendingToolCall = true;
      break;
    }

    // Not a tool_call tag — output the `<`.
    textBuffer += '<';
    i = openIdx + 1;
  }

  // Flush remaining text.
  flushText();

  const displayText = segments
    .filter((s): s is { type: 'text'; text: string } => s.type === 'text')
    .map((s) => s.text)
    .join('');

  return { segments, displayText, toolCalls, hasPendingToolCall };
}

/** Check if string is a prefix of `<tool_call` or `</tool_call>`. */
function isPartialToolCallPrefix(s: string): boolean {
  const candidates = [TOOL_CALL_OPEN, TOOL_CALL_CLOSE];
  for (const candidate of candidates) {
    if (s.length < candidate.length && candidate.startsWith(s)) {
      return true;
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// Native Mode Synthesis
// ---------------------------------------------------------------------------

/**
 * Synthesize a StreamContentResult for Native mode tool calls.
 *
 * In Native mode, the LLM does NOT emit `<tool_call>` XML tags. Instead, it
 * natively outputs tool parameters, which the backend provides as separate
 * objects. The backend wraps each intermediate iteration's text in
 * `<think>...</think>` tags so the frontend can distinguish iteration
 * boundaries from the final conclusion text.
 *
 * This function finds all **paired** `<think>...</think>` blocks, creates one
 * text segment per block, interleaves tool calls chronologically (one per
 * iteration), and leaves the post-think trailing text as the conclusion.
 *
 * Using paired matching instead of positional `<think>` splitting avoids
 * false positives from inline mentions such as `` `<think>` `` in markdown.
 */
export function synthesizeNativeStreamResult(
  content: string,
  nativeToolCalls: Array<{ name: string; arguments?: string }>
): StreamContentResult | null {
  if (nativeToolCalls.length === 0 && !content.includes('<think>')) return null;

  const toolCalls: ParsedToolCall[] = nativeToolCalls.map((tc) => ({
    name: tc.name,
    arguments: tc.arguments ?? '',
    startIndex: 0,
  }));

  // Find all paired <think>...</think> blocks.
  // Paired matching correctly ignores inline mentions like `<think>` that
  // have no corresponding closing tag.
  const THINK_OPEN = '<think>';
  const THINK_CLOSE = '</think>';
  const thinkBlocks: { start: number; end: number }[] = [];
  let searchStart = 0;
  while (searchStart < content.length) {
    const openIdx = content.indexOf(THINK_OPEN, searchStart);
    if (openIdx < 0) break;
    const closeIdx = content.indexOf(THINK_CLOSE, openIdx + THINK_OPEN.length);
    if (closeIdx < 0) break; // unclosed tag -- stop
    const blockEnd = closeIdx + THINK_CLOSE.length;
    thinkBlocks.push({ start: openIdx, end: blockEnd });
    searchStart = blockEnd;
  }

  const segments: ContentSegment[] = [];

  // No paired think blocks found -- simple: text + tool calls.
  if (thinkBlocks.length === 0) {
    if (content.trim()) segments.push({ type: 'text', text: content });
    toolCalls.forEach((tc) => segments.push({ type: 'tool_call', toolCall: tc }));
    return { segments, displayText: content, toolCalls, hasPendingToolCall: false };
  }

  let toolIdx = 0;

  // Pre-think text (text before the first <think> block).
  // In old-format data this is iteration-1 reasoning without <think> wrapper.
  if (thinkBlocks[0].start > 0) {
    const preText = content.slice(0, thinkBlocks[0].start);
    if (preText.trim()) {
      segments.push({ type: 'text', text: preText });
      // This text represents an iteration's reasoning -- pair with a tool call.
      if (toolIdx < toolCalls.length) {
        segments.push({ type: 'tool_call', toolCall: toolCalls[toolIdx++] });
      }
    }
  }

  // Each <think>...</think> block represents one LLM iteration's reasoning.
  for (let i = 0; i < thinkBlocks.length; i++) {
    const block = thinkBlocks[i];

    // Text between consecutive blocks (rare but handle gracefully).
    if (i > 0) {
      const gap = content.slice(thinkBlocks[i - 1].end, block.start);
      if (gap.trim()) {
        segments.push({ type: 'text', text: gap });
      }
    }

    // The think block text (including tags, for extractThinkTags to process).
    segments.push({ type: 'text', text: content.slice(block.start, block.end) });

    // Pair with a tool call.
    if (toolIdx < toolCalls.length) {
      segments.push({ type: 'tool_call', toolCall: toolCalls[toolIdx++] });
    }
  }

  // Post-think text (after the last </think> block) -- conclusion / final answer.
  const lastEnd = thinkBlocks[thinkBlocks.length - 1].end;
  if (lastEnd < content.length) {
    const postText = content.slice(lastEnd);
    // Place any remaining tool calls before the conclusion.
    while (toolIdx < toolCalls.length) {
      segments.push({ type: 'tool_call', toolCall: toolCalls[toolIdx++] });
    }
    if (postText.trim()) {
      segments.push({ type: 'text', text: postText });
    }
  } else {
    // No text after last think block -- place remaining tool calls at the end.
    while (toolIdx < toolCalls.length) {
      segments.push({ type: 'tool_call', toolCall: toolCalls[toolIdx++] });
    }
  }

  return { segments, displayText: content, toolCalls, hasPendingToolCall: false };
}
