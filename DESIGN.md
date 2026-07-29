---
version: alpha
name: Y-Agent
description: A quiet engineering control console for deliberate, observable agent work.
colors:
  dark-surface-primary: "#0f0f0f"
  dark-surface-secondary: "#141414"
  dark-surface-tertiary: "#1c1c1c"
  dark-surface-hover: "rgba(255, 255, 255, 0.01)"
  dark-surface-active: "rgba(255, 255, 255, 0.06)"
  dark-surface-code: "#1a1a1a"
  dark-text-primary: "#e8e6e1"
  dark-text-secondary: "#8a8680"
  dark-text-muted: "#555250"
  dark-border: "rgba(255, 255, 255, 0.06)"
  dark-border-focus: "rgba(255, 255, 255, 0.15)"
  dark-accent: "#c8b560"
  dark-accent-hover: "#d4c26e"
  dark-accent-subtle: "rgba(200, 181, 96, 0.10)"
  dark-accent-glow: "rgba(200, 181, 96, 0.15)"
  dark-accent-contrast: "#0f0f0f"
  light-surface-primary: "#ffffff"
  light-surface-secondary: "#f5f4f1"
  light-surface-tertiary: "#edecea"
  light-surface-hover: "rgba(0, 0, 0, 0.01)"
  light-surface-active: "rgba(0, 0, 0, 0.06)"
  light-surface-code: "#f1efed"
  light-text-primary: "#1a1917"
  light-text-secondary: "#6b6560"
  light-text-muted: "#9c9894"
  light-border: "rgba(0, 0, 0, 0.07)"
  light-border-focus: "rgba(0, 0, 0, 0.18)"
  light-accent: "#9a7c2a"
  light-accent-hover: "#7e6420"
  light-accent-subtle: "rgba(154, 124, 42, 0.08)"
  light-accent-glow: "rgba(154, 124, 42, 0.12)"
  light-accent-contrast: "#ffffff"
  success-dark: "#6fcf97"
  success-subtle-dark: "rgba(111, 207, 151, 0.08)"
  success-border-dark: "rgba(111, 207, 151, 0.20)"
  success-light: "#3a9d6b"
  success-subtle-light: "rgba(58, 157, 107, 0.08)"
  success-border-light: "rgba(58, 157, 107, 0.20)"
  error-dark: "#e57373"
  error-subtle-dark: "rgba(229, 115, 115, 0.08)"
  error-border-dark: "rgba(229, 115, 115, 0.20)"
  error-light: "#c0392b"
  error-subtle-light: "rgba(192, 57, 43, 0.07)"
  error-border-light: "rgba(192, 57, 43, 0.20)"
  warning-dark: "#f0c050"
  warning-subtle-dark: "rgba(240, 192, 80, 0.10)"
  warning-border-dark: "rgba(240, 192, 80, 0.25)"
  warning-light: "#c0880a"
  warning-subtle-light: "rgba(192, 136, 10, 0.10)"
  warning-border-light: "rgba(192, 136, 10, 0.25)"
  warning-contrast: "#1a1917"
  info-dark: "#60a5fa"
  info-subtle-dark: "rgba(96, 165, 250, 0.12)"
  info-border-dark: "rgba(96, 165, 250, 0.20)"
  info-light: "#2563eb"
  info-subtle-light: "rgba(37, 99, 235, 0.08)"
  info-border-light: "rgba(37, 99, 235, 0.20)"
  scrollbar-dark: "rgba(255, 255, 255, 0.10)"
  scrollbar-dark-hover: "rgba(255, 255, 255, 0.18)"
  scrollbar-light: "rgba(0, 0, 0, 0.12)"
  scrollbar-light-hover: "rgba(0, 0, 0, 0.20)"
typography:
  display:
    fontFamily: "'SF Pro Display', 'SF Pro Icons', 'Helvetica Neue', Helvetica, Arial, sans-serif"
    fontSize: "20px"
    fontWeight: 400
    lineHeight: 1.2
    letterSpacing: "0px"
  headline:
    fontFamily: "'SF Pro Display', 'SF Pro Icons', 'Helvetica Neue', Helvetica, Arial, sans-serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.3
    letterSpacing: "0px"
  title:
    fontFamily: "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: "0px"
  body:
    fontFamily: "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "0px"
  body-interface:
    fontFamily: "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "0px"
  body-compact:
    fontFamily: "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "0px"
  label:
    fontFamily: "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: 1.2
    letterSpacing: "0.08em"
  technical:
    fontFamily: "SF Mono, Fira Code, Cascadia Code, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "0px"
rounded:
  sm: "4px"
  md: "8px"
  full: "9999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  xl: "40px"
components:
  button-primary:
    backgroundColor: "{colors.dark-accent}"
    textColor: "{colors.dark-accent-contrast}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.sm}"
    padding: "0 16px"
    height: "32px"
  button-primary-light:
    backgroundColor: "{colors.light-accent}"
    textColor: "{colors.light-accent-contrast}"
  button-small:
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 12px"
    height: "28px"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.dark-text-secondary}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.sm}"
    padding: "0 16px"
    height: "32px"
  button-icon:
    backgroundColor: "transparent"
    textColor: "{colors.dark-text-muted}"
    rounded: "{rounded.sm}"
    size: "32px"
    height: "32px"
    width: "32px"
  input:
    backgroundColor: "{colors.dark-surface-primary}"
    textColor: "{colors.dark-text-primary}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.sm}"
    padding: "6px 12px"
  card:
    backgroundColor: "{colors.dark-surface-secondary}"
    textColor: "{colors.dark-text-primary}"
    rounded: "{rounded.md}"
    padding: "16px"
  dialog:
    backgroundColor: "{colors.dark-surface-primary}"
    textColor: "{colors.dark-text-primary}"
    rounded: "{rounded.md}"
    padding: "24px"
    width: "360px"
  badge:
    backgroundColor: "{colors.dark-accent-subtle}"
    textColor: "{colors.dark-accent}"
    typography: "{typography.label}"
    rounded: "{rounded.full}"
    padding: "2px 8px"
---

# Design System: Y-Agent

## Overview

**Creative North Star: "The Quiet Engineering Console"**

Y-Agent should feel like a precise workspace for developers supervising long-running,
observable agent work. The interface is calm, compact, and operational: information is
easy to scan, state changes are explicit, and decoration never competes with the task.

The visual system combines near-black or warm-white surfaces with restrained warm brass
accents. It favors continuous work surfaces, lists, toolbars, and divided settings rows
over promotional composition or a page full of floating cards. Desktop and Web share
the same React design language; host-specific window chrome or macOS vibrancy may adapt
the shell without changing component semantics.

**Key Characteristics:**

- Quiet, precise, professional, and moderately dense.
- Neutral surfaces carry structure; Muted Brass marks intent and selection.
- Compact controls, thin borders, small corners, and short transitions.
- Technical details use monospaced type and tabular numbers where comparison matters.
- Lucide icons support familiar actions without ornamental illustration.

## Colors

The palette uses warm neutrals and one deliberately scarce brass accent, with matched
dark and light themes. CSS custom properties in `crates/y-gui/src/styles/index.css`
remain the runtime source of truth; update this document when those values change.

### Primary

- **Muted Brass** (`#c8b560` dark, `#9a7c2a` light): Primary actions, selected icons,
  active tabs, progress, and small moments of emphasis.
- **Brass Hover** (`#d4c26e` dark, `#7e6420` light): Direct hover or pressed feedback
  when opacity alone is not used.
- **Brass Wash** (`rgba(200, 181, 96, 0.10)` dark,
  `rgba(154, 124, 42, 0.08)` light): Selected rows, subtle badges, and low-emphasis
  active states.

### Neutral

- **Primary Surface** (`#0f0f0f` dark, `#ffffff` light): Main content, fields, dialogs,
  and floating surfaces.
- **Secondary Surface** (`#141414` dark, `#f5f4f1` light): Sidebars, settings groups,
  composer surfaces, and grouped content.
- **Tertiary Surface** (`#1c1c1c` dark, `#edecea` light): Hovered controls and nested
  controls that need separation from a secondary surface.
- **Code Surface** (`#1a1a1a` dark, `#f1efed` light): Code, tool output, and technical
  payloads.
- **Primary Text** (`#e8e6e1` dark, `#1a1917` light): Titles, values, and essential
  content.
- **Secondary Text** (`#8a8680` dark, `#6b6560` light): Supporting copy and inactive
  controls.
- **Muted Text** (`#555250` dark, `#9c9894` light): Metadata, placeholders, and labels;
  do not use it for essential instructions at small sizes.
- **Hairline Border** (`rgba(255, 255, 255, 0.06)` dark,
  `rgba(0, 0, 0, 0.07)` light): Dividers and quiet component outlines.

### Semantic

- **Success** (`#6fcf97` dark, `#3a9d6b` light): Completed and healthy states.
- **Error** (`#e57373` dark, `#c0392b` light): Failures and destructive actions.
- **Warning** (`#f0c050` dark, `#c0880a` light): Caution and pending attention.
- **Information** (`#60a5fa` dark, `#2563eb` light): Informational selections,
  references, and non-primary highlights.

**The One Signal Rule.** Muted Brass is a signal, not a background theme. Use it on the
most important action or active state in a local region, and keep most of every screen
neutral.

## Typography

**Display Font:** SF Pro Display with Helvetica Neue and system sans-serif fallbacks.

**Body Font:** Inter with system UI fallbacks.

**Label/Mono Font:** Inter for interface labels; SF Mono, Fira Code, Cascadia Code, or
Consolas for paths, identifiers, commands, logs, tokens, and code.

The pairing is quiet and utilitarian. Display type is reserved for product identity and
compact top-level headings; the majority of the interface uses a restrained 11-15px
hierarchy suited to repeated scanning.

### Hierarchy

- **Display** (400, 20px, 1.2): Welcome identity or a rare empty-state title. Italic is
  permitted for the Y-Agent wordmark treatment only.
- **Headline** (400, 17px, 1.3): Window or main-view identity, not routine card titles.
- **Title** (600, 15px, 1.4): Dialog titles and important panel headings.
- **Body** (400, 14px, 1.55): Chat content, composer text, and sustained reading.
- **Interface Body** (400, 13px, 1.55): Primary controls, list items, and descriptions.
- **Compact Body** (400, 12px, 1.4): Forms, metadata-rich rows, menus, and buttons.
- **Label** (500, 11px, 0.08em): Section labels. Uppercase is appropriate only for
  short grouping labels, never paragraphs or action text.
- **Technical** (400, 12px, 1.55): Code, paths, IDs, logs, model data, and aligned
  numeric values. Use tabular numerals for changing counters and metrics.

**The Working Type Rule.** Do not scale text with viewport width or introduce oversized
marketing typography. Use weight, neutral color hierarchy, and spacing before adding a
new font size. Letter spacing is `0` for normal text; only short uppercase labels use
positive tracking.

## Layout

The application is a full-viewport operational shell. A fixed navigation sidebar is
normally `240px` wide (`200px` in narrow variants), separated from a flexible main
panel by a one-pixel border. Main headers are `52px` high with `24px` horizontal
padding. Dense sidebars use `6-8px` outer padding and `7px 10px` navigation rows.

Use the `4 / 8 / 16 / 24 / 40px` spacing scale. `4px` handles icon and inline gaps,
`8px` groups controls, `16px` is the default component or row inset, `24px` separates
page regions, and `40px` is reserved for empty or welcome states. Prefer `16-24px`
page padding and avoid isolated values unless a fixed-format control requires them.

Operational views should use full-width bands, split panes, tables, lists, or responsive
grids rather than decorative card stacks. Constrain fixed-format UI with explicit
tracks, minimum widths, and overflow behavior. Existing complex two-column editors
collapse to one column around `980-1180px`; narrow tool displays may simplify below
`640px`. On small widths, preserve actions and state before secondary metadata.

## Elevation & Depth

Depth is primarily structural: adjacent neutral surface tones, hairline borders, and
dividers establish hierarchy. Resting page sections remain flat. Shadows are reserved
for content that truly sits above the work surface, including dialogs, dropdowns,
popovers, tooltips, context menus, and the chat composer.

### Shadow Vocabulary

- **Low Lift** (`0 1px 3px rgba(0, 0, 0, 0.45)` dark;
  `0 1px 3px rgba(0, 0, 0, 0.07)` light): Small controls only when a border is
  insufficient.
- **Floating Layer** (`0 4px 16px rgba(0, 0, 0, 0.55)` dark;
  `0 4px 16px rgba(0, 0, 0, 0.09)` light): Menus, compact popovers, and tooltips.
- **Modal Layer** (`0 16px 40px rgba(0, 0, 0, 0.65)` dark;
  `0 16px 40px rgba(0, 0, 0, 0.12)` light): Dialogs and blocking overlays.
- **Overlay Backdrop** (`rgba(0, 0, 0, 0.5)` with `4px` backdrop blur): Modal
  isolation only.

**The Flat-by-Default Rule.** A shadow must communicate actual stacking or transient
focus. Do not add shadows to ordinary page sections or every repeated item.

## Shapes

The shape language is compact and engineered. The default corner is `4px` for buttons,
fields, menu items, inline tokens, and small controls. Use `8px` for grouped containers,
cards, popovers, and dialogs. Pill geometry is reserved for badges, statuses, chips,
switch tracks, and circular indicators.

The runtime uses the `4px / 8px` system without a larger application radius. Do not
introduce `10px` or `12px` exceptions for prominent surfaces. Borders are one pixel and
low contrast. Never nest a rounded card inside another card when a divider, plain group,
or surface shift can express the relationship.

## Components

### Buttons

Buttons are compact, quiet, and decisive.

- **Shape:** `4px` radius. Text buttons are `28px` small or `32px` medium; icon buttons
  are stable `28px` or `32px` squares.
- **Primary:** Muted Brass background, theme-specific contrast text, `0 16px` medium
  padding, and at most one primary action per local action group.
- **Ghost:** Transparent background with secondary text. Hover moves to primary text
  and a neutral hover surface or hairline border.
- **Outline:** Primary surface, hairline border, and secondary text; use where a visible
  boundary matters but the action is not primary.
- **Danger / Warning:** Use semantic colors only for actions with matching meaning.
- **Hover / Focus:** Use `100-150ms` color, background, border, opacity, or subtle scale
  transitions. Primary hover may use `0.85` opacity. Keyboard focus must remain visible;
  use the accent ring for checkable controls and the focus border for fields.
- **Disabled:** Reduce opacity to roughly `0.5`, remove pointer interaction, and keep the
  label readable. Loading actions must not resize the button.

### Chips and Badges

- **Style:** Pill-shaped, one-line, and visually small: `9-10px` text with `2px 8px`
  padding. Use a subtle semantic wash, matching text, and a low-contrast border.
- **State:** Reserve chips for compact metadata, filters, mentions, and statuses. Do not
  use pill-shaped text as a substitute for a normal command button.

### Cards and Containers

- **Corner Style:** `8px` maximum for new grouped surfaces.
- **Background:** Secondary surface for grouped settings or embedded work regions;
  primary surface for overlays and fields.
- **Shadow Strategy:** Flat at rest. Use the elevation vocabulary only when floating.
- **Border:** One-pixel Hairline Border. Prefer sibling dividers inside grouped rows.
- **Internal Padding:** `16px` default, `24px` for dialogs, and `8-12px` for dense rows.

### Inputs and Fields

- **Style:** Primary surface, one-pixel Hairline Border, `4px` radius, 12px body text,
  and `6px 12px` padding. Textareas use `8px 12px`, `1.55-1.65` line height, and
  vertical resize when appropriate.
- **Focus:** Shift the border to the theme's focus border without changing geometry.
- **Placeholder:** Muted text. Labels remain outside the field unless the established
  composer pattern requires inline placeholder text.
- **Error / Disabled:** Error uses the semantic error color plus a restrained wash;
  disabled controls retain their shape and use reduced opacity.

### Navigation

- **Style:** Fixed-width sidebar, 13px/500 item labels, 18px icons, `7px 10px` rows,
  `4px` target radius for new items, and a two-pixel vertical rhythm.
- **States:** Hover uses a subtle wash. Active items use a neutral active surface and
  border; the active icon may use Muted Brass. Text truncates with an ellipsis rather
  than widening the sidebar.
- **Mobile / Narrow:** Preserve a single maintained shared UI. Collapse or hide the
  sidebar through the shell rather than creating a duplicate navigation system.

### Dialogs, Menus, and Tooltips

- **Dialogs:** `360 / 480 / 640 / 960px` width presets, `8px` radius, `24px` padding,
  one-pixel border, and Modal Layer shadow. Always cap width at viewport minus `32px`.
- **Menus / Popovers:** Primary surface, `8px` radius, `4px` internal inset, and Floating
  Layer shadow. Items use a `4px` radius and compact 12px text.
- **Tooltips:** `4px` radius, `10px 4px` padding, 11px secondary text, and a `300ms`
  default delay. Tooltips name unfamiliar icon actions; they do not carry workflows.

### Agent Activity and Tool Output

Execution state is a signature Y-Agent surface. Keep tool calls, plans, diagnostics,
background tasks, and permissions structured and inspectable. Use monospaced values,
semantic status colors, stable columns, and compact expandable rows. Motion may indicate
active work, but it must not obscure state or shift surrounding layout.

## Do's and Don'ts

- **Do** use shared components from `crates/y-gui/src/components/ui` before introducing
  screen-specific buttons, fields, badges, dialogs, tabs, popovers, or tooltips.
- **Do** use CSS custom properties or UnoCSS semantic aliases instead of hard-coded
  theme colors in new components.
- **Do** keep normal text letter spacing at `0`; use `0.06-0.08em` only for short,
  uppercase grouping labels.
- **Do** use `4px` corners for controls and `8px` for containers and overlays.
- **Do** preserve stable control dimensions, truncation, and overflow behavior across
  desktop and narrow layouts.
- **Do** use Lucide icons for familiar actions and provide tooltips for ambiguous icons.
- **Don't** introduce `10px` or `12px` radii for new application surfaces.
- **Don't** create marketing-style hero layouts, decorative gradients, glow orbs, or
  oversized typography inside the operational application.
- **Don't** put cards inside cards or turn full page sections into floating cards.
- **Don't** use Muted Brass on large background areas or to decorate unrelated content.
- **Don't** communicate status by color alone; pair it with text, an icon, or structure.
- **Don't** fork desktop and Web visual implementations; host differences belong in
  transport, platform, or shell adapters.
