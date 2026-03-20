// Stream content processor for lazy output rendering.
//
// Parses `<tool_call>...</tool_call>` XML blocks from streaming LLM text
// into structured data. Text segments are returned for markdown rendering,
// tool call blocks are returned as structured data for card rendering.

const TOOL_CALL_TAG = 'tool_call';
const TOOL_CALL_OPEN = `<${TOOL_CALL_TAG}`;
const TOOL_CALL_CLOSE = `</${TOOL_CALL_TAG}>`;

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
// Main processor
// ---------------------------------------------------------------------------

/**
 * Process raw LLM content to produce display-safe segments.
 *
 * - Parses complete `<tool_call>...</tool_call>` blocks into structured data.
 * - Buffers any trailing partial `<tool_call>` tag so it is not shown.
 * - Returns ordered segments (text + tool_call) for rendering.
 *
 * Pure function applied to the full accumulated content.
 */
export function processStreamContent(raw: string): StreamContentResult {
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

  while (i < raw.length) {
    const openIdx = raw.indexOf('<', i);

    if (openIdx < 0) {
      textBuffer += raw.slice(i);
      break;
    }

    // Add text before the `<`.
    textBuffer += raw.slice(i, openIdx);

    const remaining = raw.slice(openIdx);

    if (remaining.startsWith(TOOL_CALL_OPEN)) {
      // Look for the matching closing tag.
      const closeIdx = raw.indexOf(TOOL_CALL_CLOSE, openIdx);
      if (closeIdx >= 0) {
        // Complete tool_call block — parse it.
        const blockEnd = closeIdx + TOOL_CALL_CLOSE.length;
        const block = raw.slice(openIdx, blockEnd);

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
    if (isPartialToolCallPrefix(remaining) && openIdx + remaining.length === raw.length) {
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
