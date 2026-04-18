# Storage & Sessions

y-agent uses a multi-backend storage architecture: SQLite (WAL mode) for operational state, JSONL files for transcripts, Qdrant for vectors, and PostgreSQL for analytics.

## Storage Architecture

```mermaid
graph TB
    subgraph SQLite["SQLite (WAL mode)"]
        SS["SqliteSessionStore<br/>Session metadata"]
        CMS["SqliteChatMessageStore<br/>Chat messages + status"]
        CCS["SqliteChatCheckpointStore<br/>Turn-level checkpoints"]
        CS["SqliteCheckpointStorage<br/>Workflow checkpoints"]
        SCS["SqliteScheduleStore<br/>Schedules + execution"]
        WS["SqliteWorkflowStore<br/>Workflow templates"]
        PMS["SqliteProviderMetricsStore<br/>Provider performance"]
    end

    subgraph JSONL["JSONL Files"]
        JTS["JsonlTranscriptStore<br/>Context transcripts (LLM-facing)"]
        JDS["JsonlDisplayTranscriptStore<br/>Display transcripts (UI-facing)"]
        SR["Skill Reflog<br/>Version history"]
    end

    subgraph FileJournal["File Journal (3-tier)"]
        T1["Inline SQLite BLOB<br/>(< 256KB)"]
        T2["External Blob Files<br/>(256KB - 10MB)"]
        T3["Git Refs<br/>(tracked files)"]
    end

    subgraph Optional["Optional Backends"]
        QD["Qdrant<br/>LTM vectors<br/>Knowledge embeddings"]
        PG["PostgreSQL<br/>Diagnostics traces<br/>Cost analytics"]
    end
```

### Connection Pool

`create_pool()` in `y-storage/src/lib.rs` creates a `sqlx::SqlitePool` configured for WAL mode:

- Write-Ahead Logging for concurrent read/write
- Connection pooling via sqlx
- Embedded migrations via `run_embedded_migrations()`
- Database files stored in `data/` directory

## Session Management

### Session Tree

Sessions form a tree structure supporting branching and sub-agent isolation:

```mermaid
graph TB
    M["Main Session<br/>type: Main<br/>depth: 0"] --> C1["Child Session<br/>type: Child<br/>depth: 1"]
    M --> B1["Branch Session<br/>type: Branch<br/>depth: 1"]
    M --> SA1["SubAgent Session<br/>type: SubAgent<br/>depth: 1"]
    SA1 --> SA2["SubAgent Session<br/>type: SubAgent<br/>depth: 2"]
    B1 --> B2["Branch Session<br/>type: Branch<br/>depth: 2"]
```

### Session Types

| Type | Purpose | Created By |
|------|---------|-----------|
| `Main` | Top-level user session | `SessionManager::create_session()` |
| `Child` | Child of a main session | Explicit child creation |
| `Branch` | Fork of an existing session | `SessionManager::fork_session()` |
| `Ephemeral` | Temporary session (no persistence) | Internal operations |
| `SubAgent` | Isolated session for delegated agents | `TaskDelegationOrchestrator` |
| `Canonical` | Canonical version of a session | `CanonicalSessionManager` |

### Session State Machine

```mermaid
stateDiagram-v2
    [*] --> Active: create
    Active --> Paused: pause
    Paused --> Active: resume
    Active --> Archived: archive
    Paused --> Archived: archive
    Active --> Merged: merge
    Active --> Tombstone: delete
    Paused --> Tombstone: delete
    Archived --> Tombstone: delete
```

### Session Lifecycle

**Creation:**
1. Allocate `SessionId` (UUID-based string)
2. Write `Session` record to SQLite
3. Create two transcript records: Context (LLM-facing) and Display (UI-facing)

**Message Append (dual-transcript write):**
1. Write message to context transcript (used as LLM conversation history)
2. Write message to display transcript (used for UI rendering)
3. Both writes are atomic SQLite inserts

**Forking:**
1. Copy parent's context transcript up to specified message index
2. Create new `Session` with `parent_id = original`, `depth = parent.depth + 1`
3. Fork shares no live state with parent -- independent branch
4. Depth limit enforced to prevent unbounded nesting

**Sub-Agent Branching:**
1. Create child session linked to parent `session_id`
2. Child inherits `knowledge_collections` and `trust_tier`
3. Isolated message history -- no parent history visible
4. Used by `TaskDelegationOrchestrator` for delegation

## Dual Transcript System

```mermaid
graph LR
    subgraph UserMsg["User Message"]
        UM["'Fix the login bug'"]
    end

    subgraph DualWrite["Dual Transcript Write"]
        CT["Context Transcript<br/>(LLM-facing)<br/>Subject to compaction"]
        DT["Display Transcript<br/>(UI-facing)<br/>Never modified"]
    end

    subgraph Usage["Usage"]
        LLM["LLM sees context<br/>transcript as history"]
        UI["UI renders display<br/>transcript for user"]
    end

    UM --> CT
    UM --> DT
    CT --> LLM
    DT --> UI
```

### Why Two Transcripts?

| Aspect | Context Transcript | Display Transcript |
|--------|-------------------|-------------------|
| Purpose | LLM conversation history | User-visible conversation |
| Compaction | Subject to summarization | Never modified |
| Content | May be summarized/pruned | Full original messages |
| Backend | JSONL (`JsonlTranscriptStore`) | JSONL (`JsonlDisplayTranscriptStore`) |
| Read By | Context pipeline (History stage) | Presentation layer |

When context compaction triggers:
1. Context transcript is replaced with a summary message
2. Display transcript remains untouched
3. User sees full conversation history in the UI
4. LLM works with compressed history to stay within context limits

## Chat Checkpoints

Turn-level checkpointing enables conversation rewind:

```mermaid
sequenceDiagram
    participant U as User
    participant CS as ChatService
    participant CP as ChatCheckpointManager
    participant RS as RewindService

    U->>CS: Send message (turn 5)
    CS->>CP: create_checkpoint(session_id, turn=5)
    Note over CP: Snapshot:<br/>- message count<br/>- journal scope<br/>- timestamp

    CS->>CS: Execute turn (tool calls, etc.)

    U->>RS: rewind(session_id, turn=5)
    RS->>CP: get_checkpoint(session_id, turn=5)
    RS->>RS: Restore messages to checkpoint state
    RS->>RS: Rollback file changes via journal
    RS-->>U: RewindResult
```

### ChatCheckpoint Structure

```
ChatCheckpoint {
    checkpoint_id: String,
    session_id: SessionId,
    turn_number: u32,
    message_count_before: u32,     // messages at checkpoint time
    journal_scope_id: Option<Uuid>, // linked file journal scope
    invalidated: bool,              // true if checkpoint is no longer valid
}
```

## File Journal

The file journal tracks all file mutations made by tools, enabling rollback:

### Three-Tier Storage

```mermaid
flowchart TD
    A["File mutation detected"] --> B{"File size?"}
    B -->|"< 256KB"| C["Inline SQLite BLOB<br/>Fastest access"]
    B -->|"256KB - 10MB"| D["External Blob File<br/>Balanced"]
    B -->|"> 10MB or git-tracked"| E["Git Ref<br/>Leverage existing VCS"]
```

### Journal Components

| Component | Responsibility |
|-----------|---------------|
| `FileJournalMiddleware` | Intercepts FileWrite/FileCreate/FileDelete/FileMove tool calls |
| `JournalStore` | Persists file snapshots with three-tier strategy |
| `JournalScope` | Groups related file changes (per-turn or per-task) |
| `FileHistoryManager` | Creates per-session file backups at user message boundaries |
| `ConflictDetector` | Detects external file modifications between journal entries |

### Rollback Flow

```mermaid
sequenceDiagram
    participant U as User
    participant RS as RewindService
    participant JS as JournalStore
    participant CD as ConflictDetector
    participant FS as Filesystem

    U->>RS: rewind(session_id, scope_id)
    RS->>JS: get_entries(scope_id)
    JS-->>RS: Vec<JournalEntry>

    loop Each entry (reverse order)
        RS->>CD: detect_conflict(entry, current_file)
        alt No conflict
            RS->>FS: Restore original content
        else External modification detected
            RS->>RS: Record RewindConflict
            Note over RS: File modified externally<br/>since journal entry
        end
    end

    RS-->>U: RollbackReport {<br/>  restored: Vec,<br/>  conflicts: Vec,<br/>  stats: DiffStats<br/>}
```

### JournalEntry

```
JournalEntry {
    entry_id: Uuid,
    scope_id: Uuid,
    file_path: PathBuf,
    operation: FileOperation,    // Create | Write | Delete | Move
    original_content: Option<Vec<u8>>,
    original_hash: Option<String>,
    timestamp: DateTime<Utc>,
    storage_strategy: StorageStrategy,
}
```

## Diagnostics Storage

### Trace Store

Two backends behind the `TraceStore` trait:

| Backend | Use Case | Storage |
|---------|----------|---------|
| `SqliteTraceStore` | Production | SQLite (default) or PostgreSQL |
| `InMemoryTraceStore` | Testing | In-memory `HashMap` |

### DiagnosticsSubscriber

Listens on the event bus and captures runtime observations:

```mermaid
graph LR
    EB["EventBus"] --> DS["DiagnosticsSubscriber"]
    DS --> TS["TraceStore"]
    DS --> CI["CostIntelligence"]

    TS --> TR["TraceReplay<br/>(debugging)"]
    TS --> TSR["TraceSearch<br/>(querying)"]
    CI --> COST["CostService<br/>(daily summaries)"]
```

Captured data per trace:
- Session ID, agent name, trace ID
- All LLM requests and responses (with raw payloads)
- Tool calls and results
- Token usage and cost
- Timing information
- Error details

## Migration System

SQLite migrations are stored in `migrations/sqlite/` and embedded into the binary:

```
migrations/sqlite/
  001_initial.sql
  002_add_schedules.sql
  003_add_workflows.sql
  ...
```

`run_embedded_migrations()` applies pending migrations on startup. This ensures the database schema is always up to date without external migration tooling.
