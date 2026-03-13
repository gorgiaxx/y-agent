# Chat Bubble Actions and Session History Tree Plan

**Version**: v0.2
**Created**: 2026-03-12
**Updated**: 2026-03-13
**Status**: Draft (updated to match current codebase)
**Owner**: y-gui (Tauri commands + frontend), y-service (chat logic), y-storage (schema), y-core (traits)

---

## 1. Overview

This plan covers the design and phased implementation of interactive actions on
user-sent message bubbles in the GUI chat panel, and the supporting
infrastructure required to make those actions safe and recoverable.

Three actions are required on user message bubbles:

| Action | Summary |
|--------|---------|
| Copy   | Copy the raw text of the user message to the clipboard. |
| Edit   | Populate the input box with the message content; on send, roll the conversation back to the checkpoint before this message and resend as a fresh LLM call. |
| Undo   | Roll the conversation back to the checkpoint immediately before this message without resending anything. |

A future requirement introduces the concept of a **session history tree**: every
checkpoint (including undone ones) is persisted in the database so the user can
later restore an undone branch.  The restore affordance is a horizontal divider
of the form `------- restore -------` rendered in the message list at the
boundary of a rolled-back segment.

---

## 2. Concepts and Terminology

**Checkpoint**: A turn-level record linking a chat turn to the conversation
state and workspace file state.  Concretely, a checkpoint stores the
`turn_number`, the `message_count_before` the turn started (for transcript
truncation), and a `journal_scope_id` (for File Journal workspace rollback).
Defined in `y-core::session::ChatCheckpoint` and stored via
`y-storage::SqliteChatCheckpointStore`.

**Rollback**: When a user edits or undoes a message, the JSONL transcript is
truncated to `message_count_before` and workspace files are restored via the
File Journal scope.  In Phase 1 messages after the rollback point are
physically removed (not soft-deleted).  Phase 2 introduces soft-delete for
branch recovery.

**Active path**: The linear sequence of messages remaining in the JSONL
transcript after any rollback, which the user currently sees in the chat panel.

**Restore divider**: (Phase 2) A UI-only sentinel rendered at the boundary of a
rolled-back segment, with a clickable "restore" affordance.

---

## 3. Priorities

| Priority | Item |
|----------|------|
| P0 | Copy button on user message bubbles (UI only, no backend) |
| P0 | Checkpoint model: already implemented in y-core, y-storage, y-service |
| P1 | Undo action: roll active path back to previous checkpoint |
| P1 | Edit action: prepopulate input box; resend triggers undo-then-send |
| P2 | Session history tree: soft-delete tombstoned branches, persist all checkpoints |
| P2 | Restore divider UI and restore action |
| P3 | Keyboard shortcuts and accessibility for all bubble actions |
| P3 | Conflict resolution if two branches produce diverging context window tokens |

---

## 4. Phase 0 -- Copy Button (P0, UI-only)

**Scope**: `MessageBubble.tsx`, `MessageBubble.css`

The existing `ActionBar` component is rendered only for assistant/system
messages.  For user messages, a simpler `UserActionBar` is added at the bottom
of the bubble presenting the three actions.

### 4.1 Changes

#### `MessageBubble.tsx`
- Add `UserActionBar` component analogous to the existing `ActionBar`.
- `UserActionBar` receives `content`, `messageId`, and three callback props:
  `onCopy`, `onEdit`, `onUndo`.
- In `MessageBubble`, render `UserActionBar` when `isUser` is true (instead of
  no action bar).
- Pass stub handlers for Edit and Undo (console.warn) until P1 is ready.

#### `MessageBubble.css`
- Style the user action bar to appear on hover using the same pattern as the
  existing `.message-actions` rules.
- Icons: `Copy` (lucide), `Pencil` (lucide), `RotateCcw` (lucide).

### 4.2 Props threading

`MessageBubble` currently receives only `message` (via `MessageBubbleProps`).
The Edit and Undo handlers need `sessionId` and message index.  Two options:

- **Option A**: Thread `onEdit(content)` + `onUndo(messageId)` down from
  `App.tsx` -> `ChatPanel` -> `MessageBubble` as callback props.
- **Option B**: Use a React context owned by `App.tsx` or `ChatPanel`.

Option A is simpler and preferred at this stage; no context overhead.

`ChatPanel.tsx` currently receives `{ messages, isStreaming, isLoading, error }`
-- it does NOT receive `sessionId`.  The `sessionId` and `useChat` return
values are managed by `App.tsx`.  To thread callbacks, either `ChatPanel` props
must be extended or `App.tsx` must wrap callbacks and pass them through
`ChatPanel` to `MessageBubble`.

---

## 5. Phase 1 -- Checkpoint Model (P0/P1 Backend) -- ALREADY IMPLEMENTED

### 5.1 What is a checkpoint?

For the chat use-case (not the orchestrator workflow use-case), a checkpoint is
a turn-level record that links a session's message count to a File Journal
scope, enabling both conversation rollback (via transcript truncation) and
workspace file rollback (via the File Journal).

This is distinct from `orchestrator_checkpoints` which checkpoints workflow
DAG state.

The checkpoint model is already implemented across three crates:
- **`y-core::session::ChatCheckpoint`** -- struct and `ChatCheckpointStore` trait
- **`y-storage::SqliteChatCheckpointStore`** -- SQLite-backed storage
- **`y-service::ChatCheckpointManager`** -- service-layer orchestration

### 5.2 SQLite table: `chat_checkpoints` (migration 010)

Already created by `migrations/sqlite/010_chat_checkpoints.up.sql`:

```sql
CREATE TABLE IF NOT EXISTS chat_checkpoints (
    checkpoint_id         TEXT PRIMARY KEY,
    session_id            TEXT NOT NULL,
    turn_number           INTEGER NOT NULL,
    message_count_before  INTEGER NOT NULL,
    journal_scope_id      TEXT NOT NULL,
    invalidated           INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CONSTRAINT unique_session_turn UNIQUE (session_id, turn_number)
);

CREATE INDEX IF NOT EXISTS idx_chat_cp_session
    ON chat_checkpoints(session_id, turn_number DESC);
```

A checkpoint is written by `ChatCheckpointManager` (via `ChatService`) once
per user turn, before the LLM call starts.  The `message_count_before` field
captures the transcript length at that point so rollback can truncate back.

### 5.3 Rollback mechanism (Phase 1)

In Phase 1, rollback uses **transcript truncation**, not soft-delete:

1. `TranscriptStore::truncate(session_id, keep_count)` physically removes
   messages from the JSONL file, keeping only the first `keep_count` messages.
2. `ChatCheckpointStore::invalidate_after(session_id, turn_number)` marks
   later checkpoints as invalidated.
3. File Journal restores workspace files to the pre-turn state using the
   `journal_scope_id`.

This is simpler than tombstoning but means rolled-back messages are lost.
Phase 2 (history tree) will introduce soft-delete to preserve branches.

The `session_get_messages` Tauri command does not need changes for Phase 1
since truncation physically removes rolled-back messages.

### 5.4 Tauri commands (new -- to be added)

These commands must be added to `crates/y-gui/src-tauri/src/commands/chat.rs`
and registered in `crates/y-gui/src-tauri/src/lib.rs` via
`tauri::generate_handler![]`.

| Command | Signature | Description |
|---------|-----------|-------------|
| `chat_checkpoint_list` | `(session_id: String) -> Vec<ChatCheckpoint>` | Return all checkpoints for session, ordered by `turn_number DESC`. Delegates to `ChatCheckpointStore::list_by_session`. |
| `chat_undo` | `(session_id: String, target_checkpoint_id: String) -> UndoResult` | Truncate transcript to checkpoint's `message_count_before`, invalidate later checkpoints, restore files via journal scope. |
| `chat_get_messages_with_status` | `(session_id: String) -> Vec<MessageWithStatus>` | (Phase 2 only) Return all messages including soft-deleted, for history tree UI. |

`UndoResult` carries:
- `remaining_message_count: usize` -- messages remaining after truncation
- `restored_turn_number: u32` -- the turn we rolled back to
- `files_restored: u32` -- number of workspace files restored via journal

### 5.5 Service layer: undo semantics

When the user clicks Undo on message M:
1. Service resolves the checkpoint for M's turn via
   `ChatCheckpointStore::load(checkpoint_id)`.
2. Truncate the JSONL transcript to `checkpoint.message_count_before` messages
   via `TranscriptStore::truncate(session_id, message_count_before)`.
3. Invalidate all checkpoints after this turn via
   `ChatCheckpointStore::invalidate_after(session_id, turn_number)`.
4. Restore workspace files via File Journal using `journal_scope_id`.
5. Return the remaining message count.

When the user clicks Edit on message M and then sends:
1. Steps 1-4 above to roll back to before M.
2. The new message text is sent as a fresh user message (same as `chat_send`
   but starting from the rolled-back context).
3. A new checkpoint is created for the new turn; LLM is called.

The service must NOT call `chat_send` a second time for the optimistic message;
the edit flow replaces the optimistic update.

---

## 6. Phase 1 -- Frontend: Undo and Edit

### 6.1 `useChat.ts` additions

```typescript
undoMessage: (sessionId: string, messageId: string) => Promise<void>;
editMessage: (content: string) => void;  // Only populates input; no network call.
```

`undoMessage` calls `chat_undo`, then reloads messages via `loadMessages`.

`editMessage` calls a callback prop from `ChatPanel` to set the input box
value.  The input box value is lifted state in `ChatPanel` (or `App.tsx`).

### 6.2 Edit flow at send time

`InputArea.tsx` currently calls `onSend(message)` -- a callback prop provided
by `App.tsx`.  The parent component invokes `useChat.sendMessage(message,
sessionId, providerId?)` which calls the `chat_send` Tauri command.  When
sending in edit mode, the flow is:

1. Call `chat_undo` to truncate back to the relevant checkpoint.
2. Then call `chat_send` with the edited content.

The edit session state (which message is being edited, which checkpoint to undo
to) is carried in a `pendingEdit` state object in `useChat` or `App.tsx`.

If the user clears the input box or navigates away, `pendingEdit` is discarded
(edit is cancelled, undo is NOT executed -- the original conversation is
unchanged).

### 6.3 UI affordances

- While `pendingEdit` is set, the input box shows a banner: "Editing message --
  sending will undo context to this point."
- The Edit button on the bubble becomes highlighted to signal active edit mode.
- An "x" icon in the banner cancels edit mode.

---

## 7. Phase 2 -- Session History Tree (P2)

### 7.1 Data model

All messages (including tombstoned) and all checkpoints are retained.  The
branch structure is implicit: a set of messages sharing the same session_id can
form multiple non-overlapping subsequences.

For UI purposes, a "branch point" is any checkpoint where a subsequent
tombstoning event occurred.  The UI renders a restore divider at each such
point in the timeline.

### 7.2 Restore divider rendering

`ChatPanel` fetches both active messages and tombstoned segments via
`chat_get_messages_with_status`.  The component assembles a display list:

```
[active messages up to branch point]
[RestoreDivider: checkpoint_id, tombstoned_message_count]
[active messages continuing...]
```

The `RestoreDivider` component is a horizontal rule with a clickable "restore"
label centered inside it.

### 7.3 Restore affordance interaction

Clicking "restore" on a divider:
1. Calls `chat_restore_branch(session_id, checkpoint_id)`.
2. The current active path after `checkpoint_id` is tombstoned (swapping
   branches).
3. The previously tombstoned branch becomes active.
4. Messages are reloaded.

After restore, the former active branch becomes the new tombstoned branch
behind a new restore divider.  This preserves full bidirectional recoverability.

### 7.4 New Tauri commands (Phase 2)

| Command | Signature | Description |
|---------|-----------|-------------|
| `chat_restore_branch` | `(session_id: String, checkpoint_id: String) -> RestoreResult` | Swap active and tombstoned branches at checkpoint boundary. |

### 7.5 SQLite changes for Phase 2

The JSONL transcript approach becomes unwieldy for tree queries.  In Phase 2,
messages are migrated to SQLite.  The next available migration number is
`012` (migrations up to `011_diagnostics` already exist).

```sql
-- migrations/sqlite/012_chat_messages.up.sql
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'tombstone')),
    checkpoint_id   TEXT REFERENCES chat_checkpoints(checkpoint_id),
    model           TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    cost_usd        REAL,
    context_window  INTEGER,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_cm_session_status ON chat_messages(session_id, status);
CREATE INDEX idx_cm_session_created ON chat_messages(session_id, created_at);
```

Migration: write a migration script that reads existing JSONL transcripts and
inserts into `chat_messages`.  The JSONL files are retained as a backup but
are no longer the source of truth.

---

## 8. Phase 3 -- Keyboard Shortcuts and Accessibility (P3)

- Copy: No dedicated shortcut needed (standard Cmd+C on selected text works).
- Edit: Alt+E when a user bubble is focused.
- Undo: Alt+Z when a user bubble is focused (distinct from global undo).
- Restore divider: keyboard-focusable with Enter to activate.

All interactive elements must have `aria-label` and `role="button"` where
applicable.

---

## 9. Open Questions

1. Should a checkpoint be written for every user message even if the LLM call
   fails or is cancelled?  Current proposal: write on commit of user message,
   then again on commit of assistant response.

2. When multiple branches exist at the same checkpoint, the restore divider
   can only show one alternative at a time.  If users undo and redo multiple
   times, this creates a chain of alternating branches.  Should the UI show
   the full branch list or only the most recent alternative?

3. The JSONL transcript format is currently the operational source of truth for
   `session_get_messages`.  The Phase 2 migration to SQLite chat_messages must
   be backward-compatible.  A feature flag (`chat_messages_sqlite`) will gate
   the migration.

4. Token count for rolled-back context: when an undo happens, the context
   window stat in the status bar should update to reflect the new (shorter)
   context.  This requires the LLM message-count calculation to use the active
   message list, not the full transcript.

5. The current Phase 1 rollback physically removes messages via transcript
   truncation.  This means once undone, the old branch is lost.  Is this
   acceptable for Phase 1, or should we implement soft-delete from the start?

---

## 10. Dependency Map

```
Phase 0 (Copy UI)
  |-- no backend dependency

Phase 1 (Undo + Edit)
  |-- requires: chat_checkpoints table (migration 010 -- DONE)
  |-- requires: ChatCheckpoint struct + store trait (y-core -- DONE)
  |-- requires: SqliteChatCheckpointStore (y-storage -- DONE)
  |-- requires: ChatCheckpointManager (y-service -- DONE)
  |-- requires: chat_undo Tauri command (src-tauri/commands/chat.rs)
  |-- requires: useChat undoMessage + editMessage additions
  |-- requires: MessageBubble UserActionBar with real callbacks
  |-- requires: App.tsx / ChatPanel pendingEdit state

Phase 2 (History Tree)
  |-- requires: Phase 1 complete
  |-- requires: chat_messages SQLite table (migration 012+)
  |-- requires: chat_get_messages_with_status Tauri command
  |-- requires: chat_restore_branch Tauri command
  |-- requires: RestoreDivider component

Phase 3 (Keyboard + a11y)
  |-- requires: Phase 1 complete
```

---

## 11. Verification Plan

### Phase 0
- Render a user message bubble and verify the three buttons appear on hover.
- Click Copy and verify clipboard contains the message text.
- Edit and Undo buttons are present but show console.warn (no-op).

### Phase 1
- Send two messages to a session.  Click Undo on the second message.  Verify
  only the first message remains visible.
- Send two messages.  Click Edit on the second.  Verify input box is populated.
  Send a new message.  Verify the original second message is removed (truncated)
  and the new message appears.
- Verify a new row exists in `chat_checkpoints` for each turn (inspectable
  via SQLite CLI: `sqlite3 y-agent.db "SELECT * FROM chat_checkpoints ORDER BY turn_number DESC LIMIT 5;"`).

### Phase 2
- Undo a message.  Verify the restore divider appears.
- Click restore.  Verify the original branch reappears and the current active
  branch is tombstoned behind a new divider.
- Repeat undo/restore twice to ensure bidirectional navigation works.

### Phase 3
- Keyboard navigation: tab to a user bubble, press Alt+E, verify input box is
  populated.
- Screen reader check: verify aria-labels are announced on action buttons.

---

## 12. Files Affected Summary

| File | Phase | Change Type |
|------|-------|-------------|
| `crates/y-gui/src/components/MessageBubble.tsx` | 0, 1 | Modify (add UserActionBar) |
| `crates/y-gui/src/components/MessageBubble.css` | 0, 1 | Modify (user action bar styles) |
| `crates/y-gui/src/components/ChatPanel.tsx` | 1, 2 | Modify (thread callbacks, add RestoreDivider) |
| `crates/y-gui/src/hooks/useChat.ts` | 1, 2 | Modify (add undoMessage, editMessage) |
| `crates/y-gui/src/components/RestoreDivider.tsx` | 2 | New |
| `crates/y-gui/src/components/RestoreDivider.css` | 2 | New |
| `crates/y-gui/src-tauri/src/commands/chat.rs` | 1, 2 | Modify (add chat_undo, chat_checkpoint_list) |
| `crates/y-gui/src-tauri/src/lib.rs` | 1, 2 | Modify (register new Tauri commands) |
| `crates/y-core/src/session.rs` | -- | Already done (ChatCheckpoint, ChatCheckpointStore) |
| `crates/y-storage/src/checkpoint_chat.rs` | -- | Already done (SqliteChatCheckpointStore) |
| `crates/y-service/src/container.rs` | -- | Already done (ChatCheckpointManager) |
| `migrations/sqlite/010_chat_checkpoints.up.sql` | -- | Already done |
| `migrations/sqlite/012_chat_messages.up.sql` | 2 | New |
| `migrations/sqlite/012_chat_messages.down.sql` | 2 | New |
| `docs/standards/DATABASE_SCHEMA.md` | 2 | Modify (add chat_messages table) |
