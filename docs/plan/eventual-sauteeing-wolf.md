# Plan: Provider Selector for GUI Chat Input

## Context

The GUI currently has no way for users to choose which LLM provider handles their chat requests -- the provider pool auto-assigns. Users need a provider selector dropdown below the chat input box that lets them pin to a specific provider-id or keep "auto" mode. The StatusBar currently shows model name (e.g., "gpt-4o"); it should instead show the provider ID, and only update after the first response arrives with the new provider -- not immediately on selector change.

There is no `preferred_provider_id` concept in `RouteRequest` today, and `chat_send` does not accept a `provider_id` parameter. The `ChatCompletePayload` and `TurnResult` only carry `model` (model name), not provider ID.

---

## Changes

### 1. Core routing: Add `preferred_provider_id` to `RouteRequest`

**File:** [provider.rs](crates/y-core/src/provider.rs) (line ~229)

Add `preferred_provider_id: Option<ProviderId>` to `RouteRequest`. The `Default` derive still works (`Option` defaults to `None`).

### 2. Router: Honor `preferred_provider_id` in selection

**File:** [router.rs](crates/y-provider/src/router.rs) (in `TagBasedRouter::select()`, line ~95)

After filtering candidates (frozen/tags/priority), check `route.preferred_provider_id`:
- If `Some(id)`, find that ID among candidates. If found, return it. If not found (frozen/filtered), return `NoProviderAvailable` error.
- If `None`, fall through to existing `preferred_model` and strategy logic.

`preferred_provider_id` takes priority over `preferred_model`.

Add tests: exact match, frozen-provider-fails, None-falls-through.

### 3. Response: Add `provider_id` to `ChatResponse`

**File:** [provider.rs](crates/y-core/src/provider.rs) (`ChatResponse` struct, line ~67)

Add `pub provider_id: Option<ProviderId>` (Option to avoid breaking existing provider impls).

### 4. Pool: Populate `provider_id` on response

**File:** [pool.rs](crates/y-provider/src/pool.rs) (in `chat_completion`, after successful response)

Set `response.provider_id = Some(entry.provider.metadata().id.clone())` before returning.

### 5. Service: Propagate `provider_id` through `TurnInput` and `TurnResult`

**File:** [chat.rs](crates/y-service/src/chat.rs)

- Add `pub provider_id: Option<String>` to `TurnInput` (line ~106 area)
- Add `pub provider_id: Option<String>` to `TurnResult`
- In `execute_turn_inner`: build `RouteRequest` with `preferred_provider_id` from `input.provider_id`, track `final_provider_id` from response, include in `TurnResult`

### 6. Tauri: New `provider_list` command

**File:** [system.rs](crates/y-gui/src-tauri/src/commands/system.rs)

```rust
#[derive(Debug, Serialize, Clone)]
pub struct ProviderInfo {
    pub id: String,
    pub model: String,
    pub provider_type: String,
}

#[tauri::command]
pub async fn provider_list(state: State<'_, AppState>) -> Result<Vec<ProviderInfo>, String>
```

Calls `pool.list_metadata()` and maps to `ProviderInfo`.

Register in [lib.rs](crates/y-gui/src-tauri/src/lib.rs) invoke handler list.

### 7. Tauri: Extend `chat_send` with `provider_id`

**File:** [chat.rs](crates/y-gui/src-tauri/src/commands/chat.rs)

- Add `provider_id: Option<String>` parameter to `chat_send`
- Pass to `TurnInput { provider_id, ... }`
- Add `provider_id: Option<String>` to `ChatCompletePayload`, populate from `result.provider_id`

### 8. Frontend types  [DONE]

**File:** [types/index.ts](crates/y-gui/src/types/index.ts)

- Add `ProviderInfo` interface: `{ id: string; model: string; provider_type: string }`
- Add `provider_id?: string` to `ChatCompletePayload` and `Message`

### 9. Frontend: Update `useChat` hook  [DONE]

**File:** [useChat.ts](crates/y-gui/src/hooks/useChat.ts)

- `sendMessage` accepts third param `providerId?: string`
- Pass `providerId` to `invoke('chat_send', { message, sessionId, providerId })`
- In `chat:complete` handler, propagate `payload.provider_id` to constructed `Message`

### 10. Frontend: `ProviderSelector` component  [DONE]

**New file:** `crates/y-gui/src/components/ProviderSelector.tsx`
**New file:** `crates/y-gui/src/components/ProviderSelector.css`

Small `<select>` dropdown. Props: `providers: ProviderInfo[]`, `selectedProviderId: string`, `onSelect: (id: string) => void`, `disabled: boolean`.

Options: "Auto" first, then each provider as `"{id} ({model})"`.

Styling: 11px font, muted color, left-aligned, dark theme matching `--surface-secondary`, `--border`, `--text-muted` variables. Sits inside `.input-area` between `input-container` and `input-hint`, using a flex row with the hint right-aligned and selector left-aligned.

### 11. Frontend: Wire into `InputArea` and `App.tsx`  [DONE]

**File:** [InputArea.tsx](crates/y-gui/src/components/InputArea.tsx)

Add provider props to `InputAreaProps`. Render `ProviderSelector` inside `.input-area` between `.input-container` and `.input-hint` in a flex row (`.input-footer` with `justify-content: space-between`).

**File:** [App.tsx](crates/y-gui/src/App.tsx)

- Add state: `providers` (fetched on mount via `provider_list`), `selectedProviderId` (default `'auto'`)
- `handleSend`: pass `selectedProviderId === 'auto' ? undefined : selectedProviderId` to `sendMessage`
- StatusBar `activeModel` tracking: use `lastAssistant.provider_id ?? lastAssistant.model` (deferred update -- only changes when a response arrives)
- Pass provider props to `InputArea`

### 12. StatusBar: Rename prop for clarity  [DONE]

**File:** [StatusBar.tsx](crates/y-gui/src/components/StatusBar.tsx)

Rename `activeModel` prop to `activeProvider` for semantic accuracy. No logic changes needed -- the deferred behavior is inherent since `lastModel` only updates from message metadata.

---

## Deferred StatusBar Update (Requirement Detail)

The StatusBar update naturally defers because:
1. `selectedProviderId` state (dropdown) is separate from `lastModel`/`lastProvider` state (StatusBar)
2. `lastModel` only updates in the `useEffect` that scans `messages` for the last assistant message
3. Changing the dropdown changes `selectedProviderId` but does NOT touch `lastModel`
4. Only when a new assistant message with `provider_id` arrives does the StatusBar reflect the change

No special logic needed -- the existing architecture provides this for free.

---

## Verification

1. `cargo test -p y-provider` -- router tests for `preferred_provider_id`
2. `cargo build -p y-gui-backend` (or equivalent tauri build) -- compile check
3. Manual GUI test:
   - Launch app, verify dropdown shows "Auto" + all configured providers
   - Select a specific provider, verify StatusBar does NOT change yet
   - Send a message, verify StatusBar updates to show the provider ID
   - Switch back to "Auto", send a message, verify StatusBar shows the auto-selected provider ID
4. Test provider-not-available scenario: freeze a provider, select it, send message -- should get an error
