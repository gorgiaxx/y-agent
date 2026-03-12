# Chat Bubble Actions and Session History Tree Plan

**Version**: v0.1
**Created**: 2026-03-12
**Status**: Draft
**Owner**: y-gui, y-cli (Tauri commands), y-storage (schema)

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

**Checkpoint**: A named snapshot of the conversation state at a specific message
boundary.  Concretely, a checkpoint captures the ordered list of message IDs
(and their content hashes) that formed the LLM context at that point in time.

**Branch**: A divergence point in the message sequence.  When a user edits or
undoes a message, the subsequent messages are not deleted; they become a
soft-deleted (tombstoned) branch that can be restored.

**Active path**: The linear sequence of non-tombstoned messages that the user
currently sees in the chat panel.

**Restore divider**: A UI-only sentinel rendered between the last active message
and the first tombstoned message of a recoverable branch, with a clickable
"restore" affordance.

---

## 3. Priorities

| Priority | Item |
|----------|------|
| P0 | Copy button on user message bubbles (UI only, no backend) |
| P0 | Checkpoint model: define what a checkpoint is in the service/storage layer |
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

`MessageBubble` currently receives only `message`.  The Edit and Undo handlers
need `sessionId` and message index.  Two options:

- **Option A**: Thread `sessionId` + `onEdit(content)` + `onUndo(messageId)`
  down from `ChatPanel` via `MessageBubble`.
- **Option B**: Use a React context owned by `ChatPanel`.

Option A is simpler and preferred at this stage; no context overhead.

`ChatPanel.tsx` already receives `sessionId` and `useChat` return values.  It
will pass the callbacks down.

---

## 5. Phase 1 -- Checkpoint Model (P0/P1 Backend)

### 5.1 What is a checkpoint?

For the chat use-case (not the orchestrator workflow use-case), a checkpoint is
a lightweight record that answers: "what was the ordered list of active message
IDs at time T for session S?"

This is distinct from `orchestrator_checkpoints` which checkpoints workflow
DAG state.  A new table is introduced.

### 5.2 New SQLite table: `chat_checkpoints`

```sql
CREATE TABLE chat_checkpoints (
    id              TEXT PRIMARY KEY,         -- UUID
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    sequence        INTEGER NOT NULL,         -- Monotonically increasing per session
    message_ids     TEXT NOT NULL,            -- JSON array of message IDs in active order
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_cc_session_seq ON chat_checkpoints(session_id, sequence DESC);
```

A checkpoint is written:
- After every successful LLM completion (i.e., after the assistant message is
  committed to the transcript).
- After every user message is committed (before the LLM call starts).

### 5.3 Message soft-delete

Messages must survive undo/edit operations so branches can be restored.  The
message model (currently stored in JSONL transcripts per session) needs a
`status` field.  Two approaches:

- **Approach A**: Add `"status": "active" | "tombstone"` to the JSONL record.
  Query layer filters tombstoned records by default.
- **Approach B**: Move messages to SQLite with an explicit `status` column.

Approach B is recommended for Phase 2 (history tree).  For Phase 1, Approach A
(JSONL annotation) is sufficient and avoids a large schema migration.

The `session_get_messages` Tauri command must be updated to filter tombstoned
messages unless explicitly requested.

### 5.4 Tauri commands (new)

| Command | Signature | Description |
|---------|-----------|-------------|
| `chat_checkpoint_list` | `(session_id: String) -> Vec<Checkpoint>` | Return all checkpoints for session, ordered by `sequence DESC`. |
| `chat_undo` | `(session_id: String, target_checkpoint_id: String) -> UndoResult` | Tombstone messages after checkpoint, write new checkpoint. |
| `chat_get_messages_with_status` | `(session_id: String) -> Vec<MessageWithStatus>` | Return all messages including tombstoned, for history tree UI. |

`UndoResult` carries:
- `new_active_message_ids: Vec<String>` -- what to display
- `restored_checkpoint_sequence: i64`

### 5.5 Service layer: undo semantics

When the user clicks Undo on message M:
1. Service resolves the checkpoint immediately preceding M's checkpoint
   (i.e., `sequence = M_checkpoint.sequence - 1`).
2. Tombstone M and all messages after M in the active path.
3. Write a new checkpoint with the pre-M message IDs.
4. Return the new active message list.

When the user clicks Edit on message M and then sends:
1. Steps 1-3 above to roll back to before M.
2. The new message text is sent as a fresh user message (same as `chat_send`
   but starting from the rolled-back context).
3. A new user-message checkpoint is written after commit; LLM is called.

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

`InputArea.tsx` currently calls `sendMessage(message, sessionId)`.  When
sending in edit mode, the flow is:

1. Call `chat_undo` to tombstone back to the relevant checkpoint.
2. Then call `chat_send` with the edited content.

The edit session state (which message is being edited, which checkpoint to undo
to) is carried in a `pendingEdit` state object in `useChat` or `ChatPanel`.

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
messages are migrated to SQLite:

```sql
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'tombstone')),
    checkpoint_id   TEXT REFERENCES chat_checkpoints(id),  -- checkpoint after which this msg was active
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

---

## 10. Dependency Map

```
Phase 0 (Copy UI)
  |-- no backend dependency

Phase 1 (Undo + Edit)
  |-- requires: chat_checkpoints table (migration)
  |-- requires: JSONL status field annotation
  |-- requires: chat_undo Tauri command
  |-- requires: useChat update
  |-- requires: MessageBubble UserActionBar with real callbacks
  |-- requires: ChatPanel pendingEdit state

Phase 2 (History Tree)
  |-- requires: Phase 1 complete
  |-- requires: chat_messages SQLite table (migration)
  |-- requires: chat_get_messages_with_status command
  |-- requires: chat_restore_branch command
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
  Send a new message.  Verify the original second message is tombstoned and the
  new message appears.
- Verify a new row exists in `chat_checkpoints` for each commit (inspectable
  via SQLite CLI: `sqlite3 y-agent.db "SELECT * FROM chat_checkpoints ORDER BY sequence DESC LIMIT 5;"`).

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
| `crates/y-gui/src/components/MessageBubble.tsx` | 0, 1 | Modify |
| `crates/y-gui/src/components/MessageBubble.css` | 0, 1 | Modify |
| `crates/y-gui/src/components/ChatPanel.tsx` | 1, 2 | Modify |
| `crates/y-gui/src/hooks/useChat.ts` | 1, 2 | Modify |
| `crates/y-gui/src/components/RestoreDivider.tsx` | 2 | New |
| `crates/y-gui/src/components/RestoreDivider.css` | 2 | New |
| `crates/y-cli/src/commands/init.rs` | 1, 2 | Modify (new Tauri commands) |
| `migrations/sqlite/007_chat_checkpoints.up.sql` | 1 | New |
| `migrations/sqlite/007_chat_checkpoints.down.sql` | 1 | New |
| `migrations/sqlite/008_chat_messages.up.sql` | 2 | New |
| `migrations/sqlite/008_chat_messages.down.sql` | 2 | New |
| `docs/standards/DATABASE_SCHEMA.md` | 1, 2 | Modify (add new tables) |
