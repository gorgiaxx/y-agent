# Refactor Settings from Overlay to Main View

Convert the settings UI from a modal dialog ([SettingsOverlay](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx#732-2273)) to a full main interface view integrated into the sidebar/main-panel layout, consistent with how Skills, Knowledge, and Agents views work.

## Proposed Changes

### Sidebar Integration

#### [MODIFY] [Sidebar.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/Sidebar.tsx)

- Add `'settings'` to the [ViewType](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/Sidebar.tsx#9-10) union type
- Change the bottom Settings gear icon from `onClick={onSettingsOpen}` to `onClick={() => handleIconClick('settings')}`, making it a regular view toggle just like chat/skills/knowledge/agents
- When `activeView === 'settings'`, render the settings category list in the sidebar panel:
  - General, Providers, Session, Runtime, Browser, Storage, Hooks, Tools, Guardrails, Knowledge, Prompts, About
- Each category item uses the same `sidebar-item` styling pattern, clicking it calls `onSelectSettingsTab(tabKey)`
- Remove the `onSettingsOpen` prop since it is no longer needed
- Add new props: `activeSettingsTab: string | null`, `onSelectSettingsTab: (tab: string) => void`

---

### Settings Panel Refactoring

#### [NEW] [SettingsPanel.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsPanel.tsx)

- A new component extracted from the "content" portion of [SettingsOverlay](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx#732-2273)
- Receives `activeTab` as a prop (controlled by sidebar), instead of managing its own tab state
- Removes the `settings-backdrop`, `settings-overlay`, `settings-header` (close button), `settings-tabs` nav, and `settings-footer` (Cancel/Save buttons)
- Keeps all form logic: General, Provider tabs, Session, Runtime, Browser, TOML editor, Prompts, About
- Save button is placed inline at the top of the content area (header bar with section title + Save + Reload buttons)
- Toast notifications remain but are positioned relative to the panel instead of the overlay
- All existing helper functions ([TagChipInput](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx#135-199), [ProviderTabPanel](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx#204-449), TOML converters, etc.) are moved here as-is

#### [MODIFY] [SettingsOverlay.css](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.css) -> [SettingsPanel.css](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsPanel.css)

- Remove `.settings-backdrop` (modal overlay background)
- Remove `.settings-overlay` (modal container with fixed dimensions)
- Remove `.settings-header` (close button row)
- Remove `.settings-tabs` (left nav -- now in sidebar)
- Remove `.settings-footer` (Cancel/Save buttons row)
- Replace `.settings-body` with `.settings-panel` that fills the main content area (`flex: 1; overflow hidden`)
- `.settings-content` keeps its existing styles (flex:1, padding, overflow-y: auto)
- `.settings-toast` repositioned for the new layout (bottom of settings-content instead of absolute within overlay)
- All form styles (`.form-group`, `.pf-*`, `.provider-*`, `.toml-*`, etc.) remain unchanged

---

### App.tsx Wiring

#### [MODIFY] [App.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/App.tsx)

- Import `SettingsPanel` instead of [SettingsOverlay](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx#732-2273)
- Remove `settingsOpen` / `setSettingsOpen` state
- Add `activeSettingsTab` state (`useState<string>('general')`)
- Add `'settings'` case to the main header title logic
- Add `activeView === 'settings'` rendering block in main panel (like skills/knowledge/agents)
- Render `<SettingsPanel activeTab={activeSettingsTab} ... />` in main content area
- Update slash command handler: `case 'settings'` and `case 'model'` switch to `setActiveView('settings')` instead of `setSettingsOpen(true)`
- Pass `activeSettingsTab` and `onSelectSettingsTab={setActiveSettingsTab}` to Sidebar
- Remove `onSettingsOpen` prop from Sidebar
- Remove the `{settingsOpen && <SettingsOverlay ... />}` block entirely

#### [DELETE] [SettingsOverlay.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/SettingsOverlay.tsx)

Old overlay component, fully replaced by `SettingsPanel.tsx`.

---

## Verification Plan

### Automated Check

```bash
cd /Users/gorgias/Projects/y-agent/crates/y-gui && npm run build
```

TypeScript build must pass with zero errors, confirming all prop changes and type updates are consistent.

### Manual Verification

Please test in the running GUI app:

1. Click the Settings gear icon in the sidebar -- it should open a "settings" view (no dialog)
2. The sidebar panel should show the settings category list (General, Providers, Session, ...)
3. Click each category -- the right main panel should show the corresponding settings form
4. Edit settings and click Save -- changes should persist
5. The `/settings` slash command should navigate to the settings view
6. Clicking another sidebar icon (Chat, Skills, etc.) should navigate away from settings normally
