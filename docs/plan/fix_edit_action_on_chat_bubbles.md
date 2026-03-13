# Fix: Edit Action on User Chat Bubbles

## Problem

Clicking **Edit** on a user message bubble only copies text to the input box. When the user sends:

1. The original bubble and its assistant response remain visible
2. The LLM receives the full un-truncated history (including the old message + response)
3. The checkpoint lookup grabs `checkpoints[0]` (latest turn) instead of the checkpoint for the *edited* message

## Root Causes

### 1. `pendingEdit` lacks message index

`handleEditMessage` in `App.tsx` stores `{ messageId, content }` but `messageId` is a synthetic `edit-{timestamp}` -- it carries no information about *which turn* to roll back to.

### 2. Checkpoint targeting is wrong

`handleSend` calls `chat_checkpoint_list` and blindly uses `checkpoints[0]` (the latest checkpoint). If the user edits message 2 of 6, this rolls back to the *last* turn, not message 2's turn.

### 3. UI messages are not truncated

After the undo succeeds, `loadMessages` is called inside `undoMessage`. But `sendMessage` then *appends* an optimistic user message to whatever `messages` state exists. The old bubbles that should have been removed by the undo may still be visible due to a race between `loadMessages` and the optimistic append.

## Proposed Changes

All changes are frontend-only. The backend `chat_undo`, `chat_checkpoint_list`, and `chat_send` commands already work correctly.

---

### [MODIFY] [MessageBubble.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/MessageBubble.tsx)

Change `onEdit` callback signature to include the message index:

```diff
-  onEdit?: (content: string) => void;
+  onEdit?: (content: string, messageIndex: number) => void;
```

`UserActionBar` passes the index when calling `onEdit`.

### [MODIFY] [ChatPanel.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/ChatPanel.tsx)

Thread the message index from the `messages.map()` loop:

```diff
-  <MessageBubble key={msg.id} message={msg} onEdit={onEditMessage} .../>
+  <MessageBubble key={msg.id} message={msg} onEdit={(content) => onEditMessage?.(content, idx)} .../>
```

Update the `onEditMessage` prop type to accept `(content: string, messageIndex: number)`.

### [MODIFY] [App.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/App.tsx)

**1. Store message index in `pendingEdit`:**

```diff
- const [pendingEdit, setPendingEdit] = useState<{ messageId: string; content: string } | null>(null);
+ const [pendingEdit, setPendingEdit] = useState<{ messageId: string; content: string; messageIndex: number } | null>(null);
```

**2. Fix `handleEditMessage`** to accept and store the index:

```diff
- const handleEditMessage = useCallback((content: string) => {
-   setPendingEdit({ messageId: `edit-${Date.now()}`, content });
+ const handleEditMessage = useCallback((content: string, messageIndex: number) => {
+   setPendingEdit({ messageId: `edit-${Date.now()}`, content, messageIndex });
  }, []);
```

**3. Fix `handleSend` edit path** to find the correct checkpoint by matching `message_count_before` to the edited message's index:

```diff
  if (pendingEdit) {
    try {
      const checkpoints = await invoke<ChatCheckpointInfo[]>('chat_checkpoint_list', {
        sessionId: sid,
      });
-     if (checkpoints.length > 0) {
-       await undoMessage(sid, checkpoints[0].checkpoint_id);
-     }
+     // Find the checkpoint whose message_count_before matches the
+     // edited message's position. The checkpoint records how many messages
+     // existed *before* the turn started, so message at index N was produced
+     // by the checkpoint with message_count_before == N.
+     const targetCp = checkpoints.find(
+       (cp) => cp.message_count_before === pendingEdit.messageIndex
+     );
+     if (targetCp) {
+       await undoMessage(sid, targetCp.checkpoint_id);
+     } else if (checkpoints.length > 0) {
+       // Fallback: use latest checkpoint if exact match not found.
+       await undoMessage(sid, checkpoints[0].checkpoint_id);
+     }
    } catch (e) { ... }
    setPendingEdit(null);
  }
```

The `undoMessage` in `useChat.ts` already calls `loadMessages(sessionId)` after undo, which reloads the truncated message list. The subsequent `sendMessage` call then optimistically appends to the *already-truncated* list, which is correct.

### [MODIFY] [InputArea.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/InputArea.tsx)

The `pendingEdit` type gains `messageIndex` but `InputArea` only reads `content` -- no changes needed to the component itself, only its interface type.

## Verification

1. Open GUI, send 3 messages (get 3 assistant responses = 6 messages total)
2. Click Edit on message 2 (the second user bubble)
3. Verify: edit banner appears, textarea shows the message text
4. Modify the text and send
5. Verify: messages 2-6 disappear, new message + new assistant response appear
6. The JSONL transcript should only contain: message 1, assistant response 1, new edited message, new assistant response
