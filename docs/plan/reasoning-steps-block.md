# Plan: Collapsible Reasoning Process Block

## Problem

Multi-iteration assistant messages (LLM call -> tool calls -> LLM call -> ...) currently:
1. After completion: only show the final LLM call's text; intermediate iteration texts are lost
2. Tool call cards are appended at the bottom, out of chronological order
3. No way to see the reasoning process (intermediate LLM outputs + tool executions)

## Desired Behavior

Display order in each assistant message bubble (top to bottom):
1. **ThinkingBlock** (existing) - reasoning_content / `<think>` tags
2. **ReasoningStepsBlock** (new, collapsible) - all intermediate LLM calls + tool results in chronological order
3. **Final answer** - only the last LLM iteration's text

The ReasoningStepsBlock is collapsed by default for completed messages, expanded during streaming.
Only rendered when there are multiple iterations (at least one intermediate LLM call with tool calls).

## Architecture

### Data Flow

**During streaming (live)**:
- ChatBus forwards `llm_response` progress events (new) alongside existing `tool_result` events
- `useChat` accumulates per-session `reasoningSteps` (interleaved llm_response + tool_result events)
- Tracks `contentOffsets` (character index in streaming content when each intermediate iteration ends)
- MessageBubble uses live reasoningSteps + contentOffsets to split accumulated content into intermediate vs final

**After completion (historical)**:
- Backend persists `reasoning_steps` array in assistant message metadata
- Built from `result.new_messages` (all intermediate assistant + tool messages from execution loop)
- MessageBubble reads `metadata.reasoning_steps` and renders the collapsible block
- Main content is `result.content` (final iteration text only) -- no splitting needed

### Data Structure

```typescript
// Shared type for both live and persisted reasoning steps
interface ReasoningStep {
  type: 'llm_call' | 'tool_result';
  // llm_call fields:
  contentPreview?: string;   // intermediate LLM response text (first 500 chars for metadata)
  toolCalls?: string[];      // tool names requested
  // tool_result fields:
  name?: string;
  success?: boolean;
  durationMs?: number;
  resultPreview?: string;
}
```

## Changes

### 1. Backend: `crates/y-service/src/chat.rs`

In `execute_turn`, after building `tool_results_meta`, also build `reasoning_steps`:

```rust
let mut reasoning_steps: Vec<serde_json::Value> = Vec::new();
let mut tool_idx = 0;
for msg in &result.new_messages {
    match msg.role {
        Role::Assistant => {
            // ALL assistant messages in result.new_messages are intermediate
            // (the final one is built separately and pushed after)
            let preview_end = msg.content.floor_char_boundary(500);
            let tool_names: Vec<&str> = msg.tool_calls.iter()
                .map(|tc| tc.function.name.as_str())
                .collect();
            reasoning_steps.push(serde_json::json!({
                "type": "llm_call",
                "content_preview": &msg.content[..preview_end],
                "tool_calls": tool_names,
            }));
        }
        Role::Tool => {
            if tool_idx < result.tool_calls_executed.len() {
                let tc = &result.tool_calls_executed[tool_idx];
                let preview_end = tc.result_content.floor_char_boundary(500);
                reasoning_steps.push(serde_json::json!({
                    "type": "tool_result",
                    "name": tc.name,
                    "success": tc.success,
                    "duration_ms": tc.duration_ms,
                    "result_preview": &tc.result_content[..preview_end],
                }));
                tool_idx += 1;
            }
        }
        _ => {}
    }
}
// Only add to metadata if there were intermediate steps
if !reasoning_steps.is_empty() {
    meta["reasoning_steps"] = serde_json::Value::Array(reasoning_steps);
}
```

### 2. Frontend: `crates/y-gui/src/hooks/useChat.ts`

**Add `llm_response` to ChatBusEvent union:**
```typescript
| { type: 'llm_response'; session_id: string; iteration: number;
    tool_calls_requested: string[]; response_text?: string }
```

**Forward `llm_response` in ChatBus listener** (inside `chat:progress` handler):
```typescript
} else if (event.type === 'llm_response') {
  const session_id = chatBusState.runToSession[run_id];
  if (session_id) {
    notifyChatSubscribers({
      type: 'llm_response',
      session_id,
      iteration: event.iteration,
      tool_calls_requested: event.tool_calls_requested,
      response_text: event.response_text,
    });
  }
}
```

**New per-session state:**
- `reasoningStepsRef`: `Map<string, ReasoningStep[]>` -- accumulated reasoning steps
- `contentOffsetsRef`: `Map<string, number[]>` -- content split points
- `visibleReasoningSteps` / `visibleContentOffsets` -- for active session

**Handler for `llm_response` bus event:**
- If `event.tool_calls_requested.length > 0` (intermediate iteration):
  - Snapshot current streaming message content length as a content offset
  - Add `{ type: 'llm_call', toolCalls: event.tool_calls_requested }` step

**On `chat:started`:** Clear reasoning steps and content offsets for the session.

**On `chat:complete` / cancel:** Snapshot reasoning steps into streaming message metadata (like existing tool_results snapshot).

**Expose from hook:** `reasoningSteps: ReasoningStep[]`, `contentOffsets: number[]`

### 3. Frontend: `crates/y-gui/src/components/ChatPanel.tsx`

Pass new props to MessageBubble:
```tsx
<MessageBubble
  ...
  reasoningSteps={
    (msg.id.startsWith('streaming-') || msg.id.startsWith('cancelled-') || msg.id.startsWith('error-'))
      ? reasoningSteps : undefined
  }
  contentOffsets={
    (msg.id.startsWith('streaming-') || msg.id.startsWith('cancelled-') || msg.id.startsWith('error-'))
      ? contentOffsets : undefined
  }
/>
```

### 4. Frontend: New `crates/y-gui/src/components/ReasoningStepsBlock.tsx`

Collapsible component following the ThinkingBlock pattern:

**Props:**
```typescript
interface ReasoningStepsBlockProps {
  steps: ReasoningStep[];
  isStreaming?: boolean;
}
```

**Layout (collapsed):**
```
[icon] Reasoning Process  (N steps)  [duration]  [chevron]
```

**Layout (expanded):**
Each step rendered in order:
- `llm_call` step: Light background card with the content preview text (truncated) + "Called: tool1, tool2" badge
- `tool_result` step: Compact ToolCallCard (reuse existing component)

**Behavior:**
- Default expanded during streaming, auto-collapses when streaming ends
- Default collapsed for historical messages
- Click header to toggle

**CSS:** New `ReasoningStepsBlock.css` following ThinkingBlock.css patterns.
Use a distinct left border color (e.g. `#60a5fa` blue) to differentiate from thinking (purple).

### 5. Frontend: `crates/y-gui/src/components/MessageBubble.tsx`

**New props:**
```typescript
interface MessageBubbleProps {
  ...
  reasoningSteps?: ReasoningStep[];   // live streaming steps
  contentOffsets?: number[];          // content split points
}
```

**Compute reasoning steps (unified source):**
```typescript
const resolvedSteps = useMemo(() => {
  // Live streaming steps take priority
  if (reasoningSteps && reasoningSteps.length > 0) return reasoningSteps;
  // Fallback: persisted metadata
  const metaSteps = message.metadata?.reasoning_steps;
  if (Array.isArray(metaSteps) && metaSteps.length > 0) {
    return metaSteps.map(parseReasoningStep);
  }
  return [];
}, [reasoningSteps, message.metadata]);
```

**Content splitting for streaming:**
```typescript
const displayContent = useMemo(() => {
  if (resolvedSteps.length === 0) return effectiveContent;
  // For historical messages: content is already just the final answer
  if (!contentOffsets || contentOffsets.length === 0) return effectiveContent;
  // For streaming: content after the last offset is the final answer
  const lastOffset = contentOffsets[contentOffsets.length - 1];
  return effectiveContent.slice(lastOffset);
}, [effectiveContent, resolvedSteps, contentOffsets]);
```

**For streaming steps, inject content previews from the accumulated content:**
```typescript
// Enrich llm_call steps with actual content from the streaming message
const enrichedSteps = useMemo(() => {
  if (!contentOffsets || contentOffsets.length === 0) return resolvedSteps;
  let llmCallIdx = 0;
  return resolvedSteps.map(step => {
    if (step.type === 'llm_call') {
      const start = llmCallIdx === 0 ? 0 : contentOffsets[llmCallIdx - 1];
      const end = contentOffsets[llmCallIdx] ?? effectiveContent.length;
      llmCallIdx++;
      return { ...step, contentPreview: effectiveContent.slice(start, end) };
    }
    return step;
  });
}, [resolvedSteps, contentOffsets, effectiveContent]);
```

**Render order change:**
```tsx
{/* 1. ThinkingBlock (existing) */}
{/* 2. ReasoningStepsBlock (new -- between thinking and content) */}
{resolvedSteps.length > 0 && (
  <ReasoningStepsBlock
    steps={enrichedSteps}
    isStreaming={isStreamingMsg}
  />
)}
{/* 3. Main content (final answer only via displayContent) */}
```

**Remove old bottom-of-message tool cards rendering** (lines 526-544) when `resolvedSteps` is non-empty.
The tool results are now inside the ReasoningStepsBlock. Keep the old rendering as fallback for messages without `reasoning_steps` metadata (backward compat).

### 6. Types: `crates/y-gui/src/types/index.ts`

Add `ReasoningStep` type (or keep it local to the components that use it).

## File Summary

| File | Change Type |
|------|-------------|
| `crates/y-service/src/chat.rs` | Add `reasoning_steps` to metadata |
| `crates/y-gui/src/hooks/useChat.ts` | Forward `llm_response`, track reasoning state |
| `crates/y-gui/src/components/ChatPanel.tsx` | Pass new props |
| `crates/y-gui/src/components/ReasoningStepsBlock.tsx` | **New file** |
| `crates/y-gui/src/components/ReasoningStepsBlock.css` | **New file** |
| `crates/y-gui/src/components/MessageBubble.tsx` | Integrate block, split content |
| `crates/y-gui/src/types/index.ts` | Add ReasoningStep type |

## Quality Gates

- `cargo clippy --workspace -- -D warnings`
- `cargo check --workspace`
- `npx tsc --noEmit --skipLibCheck` (frontend type check)
- Manual verification: multi-iteration session shows reasoning block after reload
