//! Terminal-aware color theme for TUI rendering.
//!
//! macOS Terminal.app does not support truecolor (24-bit RGB) escape sequences.
//! When crossterm emits `\x1b[38;2;R;G;Bm`, Terminal.app either ignores it or
//! renders an incorrect color, causing both the "wrong colors" bug and the
//! "partial selection" visual artifact (inconsistent backgrounds between Spans
//! that have explicit bg and those that fall back to terminal default).
//!
//! This module detects terminal capabilities via `COLORTERM` and `TERM` env vars
//! and selects the appropriate color palette at startup:
//!
//! | Terminal              | `COLORTERM`   | Palette used |
//! |-----------------------|---------------|-------------|
//! | iTerm2, Alacritty... | `truecolor`   | RGB (rich)  |
//! | macOS Terminal.app    | _(empty)_     | 256-color   |
//! | xterm, screen        | other / unset | 256-color   |
//!
//! Every semantic color has both an RGB value and a 256-color `Indexed` fallback.
//! The 256-color values are chosen from the 6x6x6 color cube (indices 16-231)
//! and the 24-step grayscale ramp (indices 232-255) to closely approximate the
//! RGB originals.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;

use ratatui::style::Color;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Terminal capability detection
// ---------------------------------------------------------------------------

/// Whether the terminal advertises truecolor (24-bit RGB) support.
///
/// Detection strategy (matches what crossterm/VTE apps commonly check):
/// 1. `COLORTERM=truecolor` or `COLORTERM=24bit` -> truecolor
/// 2. Otherwise -> assume 256-color only
///
/// Terminals that set `COLORTERM=truecolor`: iTerm2, Alacritty, Kitty,
/// `WezTerm`, Ghostty, Windows Terminal, foot, etc.
///
/// macOS Terminal.app does **not** set `COLORTERM` at all.
fn terminal_supports_truecolor() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
}

// ---------------------------------------------------------------------------
// Theme struct
// ---------------------------------------------------------------------------

/// Color theme that adapts to terminal capabilities.
///
/// Provides semantic colors for every visual element in the TUI. On truecolor
/// terminals the richer RGB palette is used; on limited terminals (macOS
/// Terminal.app) the 256-color fallback indices are used instead.
///
/// Use `Theme::default()` to obtain the auto-detected singleton.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Whether the connected terminal supports truecolor.
    truecolor: bool,
    /// User-provided semantic color overrides, already resolved for the host.
    overrides: HashMap<String, Color>,
}

/// One selectable theme shown by the `/theme` picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeInfo {
    pub name: String,
    pub label: String,
    pub description: String,
    pub is_custom: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            truecolor: terminal_supports_truecolor(),
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    #[serde(default)]
    colors: BTreeMap<String, ThemeColorValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ThemeColorValue {
    Index(u8),
    Text(String),
}

/// Invalid custom theme file, name, or semantic color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError(String);

impl fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ThemeError {}

const SEMANTIC_COLORS: &[&str] = &[
    "panel_bg",
    "code_bg",
    "border_focused",
    "title",
    "text",
    "muted",
    "user_accent",
    "assistant_accent",
    "system_accent",
    "selected",
    "active",
    "normal",
    "success",
    "error",
    "warning",
    "streaming_dot",
    "think_accent",
    "think_text",
    "tool_card_accent",
    "tool_card_text",
    "blockquote",
    "hr",
    "welcome",
    "code_fg",
    "code_block_fg",
    "status_model",
    "status_path",
    "status_bar_track",
    "status_bar_normal",
    "status_bar_warn",
    "status_sep",
    "status_cost",
    "status_version",
    "status_bar_bg",
    "input_border_focused",
    "input_border_unfocused",
    "input_title",
    "cursor_fg",
    "cursor_bg",
    "cursor_unfocused",
];

#[derive(Debug, Clone, Copy)]
struct BuiltinPalette {
    background: &'static str,
    surface: &'static str,
    border: &'static str,
    primary: &'static str,
    accent: &'static str,
    text: &'static str,
    strong: &'static str,
    muted: &'static str,
    user: &'static str,
    success: &'static str,
    warning: &'static str,
    error: &'static str,
    thinking: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct BuiltinTheme {
    name: &'static str,
    label: &'static str,
    description: &'static str,
    palette: Option<BuiltinPalette>,
}

const BUILTIN_THEMES: &[BuiltinTheme] = &[
    BuiltinTheme {
        name: "default",
        label: "Y Agent Default",
        description: "The original balanced blue palette.",
        palette: None,
    },
    BuiltinTheme {
        name: "dark",
        label: "Dark",
        description: "Clean high-contrast dark colors inspired by Kimi Code.",
        palette: Some(BuiltinPalette {
            background: "#16161e",
            surface: "#282a36",
            border: "#5a5a5a",
            primary: "#4fa8ff",
            accent: "#5bc0be",
            text: "#e0e0e0",
            strong: "#f5f5f5",
            muted: "#888888",
            user: "#ffcb6b",
            success: "#4ec87e",
            warning: "#e8a838",
            error: "#e85454",
            thinking: "#bd93f9",
        }),
    },
    BuiltinTheme {
        name: "light",
        label: "Light",
        description: "A crisp light palette with accessible text contrast.",
        palette: Some(BuiltinPalette {
            background: "#fafafa",
            surface: "#f0f2f5",
            border: "#737373",
            primary: "#1565c0",
            accent: "#00838f",
            text: "#1a1a1a",
            strong: "#1a1a1a",
            muted: "#5f5f5f",
            user: "#9a4a00",
            success: "#0e7a38",
            warning: "#92660a",
            error: "#b91c1c",
            thinking: "#7c3aed",
        }),
    },
    BuiltinTheme {
        name: "nord",
        label: "Nord",
        description: "Cool arctic blues with soft, readable contrast.",
        palette: Some(BuiltinPalette {
            background: "#2e3440",
            surface: "#3b4252",
            border: "#4c566a",
            primary: "#88c0d0",
            accent: "#8fbcbb",
            text: "#eceff4",
            strong: "#ffffff",
            muted: "#a3b1c2",
            user: "#ebcb8b",
            success: "#a3be8c",
            warning: "#ebcb8b",
            error: "#bf616a",
            thinking: "#b48ead",
        }),
    },
    BuiltinTheme {
        name: "gruvbox-dark",
        label: "Gruvbox Dark",
        description: "Warm retro colors with strong code and status contrast.",
        palette: Some(BuiltinPalette {
            background: "#282828",
            surface: "#3c3836",
            border: "#665c54",
            primary: "#83a598",
            accent: "#8ec07c",
            text: "#ebdbb2",
            strong: "#fbf1c7",
            muted: "#a89984",
            user: "#fabd2f",
            success: "#b8bb26",
            warning: "#fabd2f",
            error: "#fb4934",
            thinking: "#d3869b",
        }),
    },
    BuiltinTheme {
        name: "solarized-dark",
        label: "Solarized Dark",
        description: "Low-glare cyan and blue on a deep teal background.",
        palette: Some(BuiltinPalette {
            background: "#002b36",
            surface: "#073642",
            border: "#586e75",
            primary: "#268bd2",
            accent: "#2aa198",
            text: "#eee8d5",
            strong: "#fdf6e3",
            muted: "#839496",
            user: "#b58900",
            success: "#859900",
            warning: "#b58900",
            error: "#dc322f",
            thinking: "#6c71c4",
        }),
    },
    BuiltinTheme {
        name: "solarized-light",
        label: "Solarized Light",
        description: "Low-glare dark text on Solarized's warm light base.",
        palette: Some(BuiltinPalette {
            background: "#fdf6e3",
            surface: "#eee8d5",
            border: "#93a1a1",
            primary: "#268bd2",
            accent: "#2aa198",
            text: "#073642",
            strong: "#002b36",
            muted: "#657b83",
            user: "#b58900",
            success: "#859900",
            warning: "#b58900",
            error: "#dc322f",
            thinking: "#6c71c4",
        }),
    },
    BuiltinTheme {
        name: "dracula",
        label: "Dracula",
        description: "Vivid cyan, green, and violet on charcoal.",
        palette: Some(BuiltinPalette {
            background: "#282a36",
            surface: "#343746",
            border: "#6272a4",
            primary: "#8be9fd",
            accent: "#50fa7b",
            text: "#f8f8f2",
            strong: "#ffffff",
            muted: "#a7a7a4",
            user: "#f1fa8c",
            success: "#50fa7b",
            warning: "#f1fa8c",
            error: "#ff5555",
            thinking: "#bd93f9",
        }),
    },
];

impl Theme {
    /// Load a built-in theme or a custom theme from
    /// `<config-dir>/themes/<name>.toml`.
    pub fn load(name: &str, config_dir: Option<&Path>) -> Result<Self, ThemeError> {
        Self::load_with_capability(name, config_dir, terminal_supports_truecolor())
    }

    /// List built-in themes followed by valid custom themes discovered on disk.
    ///
    /// Custom files are re-scanned every time so `/theme` sees new files
    /// without restarting the TUI. A custom file cannot shadow a built-in.
    pub fn available_themes(config_dir: Option<&Path>) -> Vec<ThemeInfo> {
        let mut themes: Vec<ThemeInfo> = BUILTIN_THEMES
            .iter()
            .map(|theme| ThemeInfo {
                name: theme.name.to_string(),
                label: theme.label.to_string(),
                description: theme.description.to_string(),
                is_custom: false,
            })
            .collect();
        let Some(config_dir) = config_dir else {
            return themes;
        };
        let directory = config_dir.join("themes");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return themes;
        };
        let mut custom_names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                    return None;
                }
                let name = path.file_stem()?.to_str()?;
                if BUILTIN_THEMES.iter().any(|theme| theme.name == name)
                    || !valid_theme_name(name)
                    || Self::load_with_capability(name, Some(config_dir), true).is_err()
                {
                    return None;
                }
                Some(name.to_string())
            })
            .collect();
        custom_names.sort_unstable();
        themes.extend(custom_names.into_iter().map(|name| ThemeInfo {
            label: format!("Custom: {name}"),
            description: "User theme loaded from the themes directory.".to_string(),
            name,
            is_custom: true,
        }));
        themes
    }

    fn load_with_capability(
        name: &str,
        config_dir: Option<&Path>,
        truecolor: bool,
    ) -> Result<Self, ThemeError> {
        if let Some(builtin) = BUILTIN_THEMES.iter().find(|theme| theme.name == name) {
            return Ok(Self {
                truecolor,
                overrides: builtin.palette.map_or_else(
                    || Ok(HashMap::new()),
                    |palette| palette_overrides(palette, truecolor),
                )?,
            });
        }
        if !valid_theme_name(name) {
            return Err(ThemeError(format!("invalid theme name `{name}`")));
        }
        let directory = config_dir.ok_or_else(|| {
            ThemeError(format!(
                "theme `{name}` requires a user configuration directory"
            ))
        })?;
        let path = directory.join("themes").join(format!("{name}.toml"));
        let source = std::fs::read_to_string(&path)
            .map_err(|error| ThemeError(format!("could not read {}: {error}", path.display())))?;
        let file: ThemeFile = toml::from_str(&source)
            .map_err(|error| ThemeError(format!("could not parse {}: {error}", path.display())))?;
        let mut overrides = HashMap::new();
        for (semantic, value) in file.colors {
            if !SEMANTIC_COLORS.contains(&semantic.as_str()) {
                return Err(ThemeError(format!(
                    "unknown semantic color `{semantic}` in {}",
                    path.display()
                )));
            }
            overrides.insert(semantic, resolve_theme_color(value, truecolor)?);
        }
        Ok(Self {
            truecolor,
            overrides,
        })
    }

    #[cfg(test)]
    fn load_for_terminal(
        name: &str,
        config_dir: &Path,
        truecolor: bool,
    ) -> Result<Self, ThemeError> {
        Self::load_with_capability(name, Some(config_dir), truecolor)
    }

    // -----------------------------------------------------------------------
    // Helper: pick RGB or Indexed
    // -----------------------------------------------------------------------

    /// Return the RGB color on truecolor terminals, or the 256-color index
    /// otherwise.
    fn color(&self, semantic: &str, rgb: Color, idx: Color) -> Color {
        if let Some(color) = self.overrides.get(semantic) {
            return *color;
        }
        match (self.truecolor, rgb, idx) {
            (true, rgb, _) => rgb,
            (false, _, idx) => idx,
        }
    }

    // -----------------------------------------------------------------------
    // Backgrounds
    // -----------------------------------------------------------------------

    /// Dark panel background.
    pub fn panel_bg(&self) -> Color {
        // RGB(22,22,30) -> grayscale index 234 (very dark blue-gray)
        self.color("panel_bg", Color::Rgb(22, 22, 30), Color::Indexed(234))
    }

    /// Code block background.
    pub fn code_bg(&self) -> Color {
        // RGB(40,42,54) -> index 235 (Dracula-ish dark)
        self.color("code_bg", Color::Rgb(40, 42, 54), Color::Indexed(235))
    }

    // -----------------------------------------------------------------------
    // Borders
    // -----------------------------------------------------------------------

    /// Focused panel border.
    pub fn border_focused(&self) -> Color {
        // RGB(120,180,255) -> index 75 (blue, 6x6x6 cube)
        self.color(
            "border_focused",
            Color::Rgb(120, 180, 255),
            Color::Indexed(75),
        )
    }

    // -----------------------------------------------------------------------
    // Text
    // -----------------------------------------------------------------------

    /// Panel title text.
    pub fn title(&self) -> Color {
        // RGB(180,180,200) -> index 252 (light gray)
        self.color("title", Color::Rgb(180, 180, 200), Color::Indexed(252))
    }

    /// Primary content text.
    pub fn text(&self) -> Color {
        // RGB(220,220,230) -> index 254 (near-white)
        self.color("text", Color::Rgb(220, 220, 230), Color::Indexed(254))
    }

    /// Muted / secondary text.
    pub fn muted(&self) -> Color {
        // RGB(100,100,120) -> index 245 (mid gray)
        self.color("muted", Color::Rgb(100, 100, 120), Color::Indexed(245))
    }

    // -----------------------------------------------------------------------
    // Role accents
    // -----------------------------------------------------------------------

    /// User role accent (green).
    pub fn user_accent(&self) -> Color {
        // RGB(130,220,130) -> index 114 (bright green)
        self.color(
            "user_accent",
            Color::Rgb(130, 220, 130),
            Color::Indexed(114),
        )
    }

    /// Assistant role accent (blue).
    pub fn assistant_accent(&self) -> Color {
        // RGB(120,180,255) -> index 75 (blue)
        self.color(
            "assistant_accent",
            Color::Rgb(120, 180, 255),
            Color::Indexed(75),
        )
    }

    /// System role accent (yellow).
    pub fn system_accent(&self) -> Color {
        // RGB(220,200,100) -> index 179 (yellow)
        self.color(
            "system_accent",
            Color::Rgb(220, 200, 100),
            Color::Indexed(179),
        )
    }

    // -----------------------------------------------------------------------
    // List / selection
    // -----------------------------------------------------------------------

    /// Selected item highlight color.
    pub fn selected(&self) -> Color {
        self.color("selected", Color::Rgb(120, 180, 255), Color::Indexed(75))
    }

    /// Active / current indicator color.
    pub fn active(&self) -> Color {
        self.color("active", Color::Rgb(130, 220, 130), Color::Indexed(114))
    }

    /// Normal (non-selected) item text.
    pub fn normal(&self) -> Color {
        self.color("normal", Color::Rgb(180, 180, 200), Color::Indexed(252))
    }

    // -----------------------------------------------------------------------
    // Status
    // -----------------------------------------------------------------------

    /// Success / done status.
    pub fn success(&self) -> Color {
        // RGB(100,200,120) -> index 114 (green)
        self.color("success", Color::Rgb(100, 200, 120), Color::Indexed(114))
    }

    /// Error status.
    pub fn error(&self) -> Color {
        // RGB(255,100,100) -> index 167 (bright red)
        self.color("error", Color::Rgb(255, 100, 100), Color::Indexed(167))
    }

    /// Warning / running / streaming status.
    pub fn warning(&self) -> Color {
        // RGB(255,200,60) -> index 179 (yellow)
        self.color("warning", Color::Rgb(255, 200, 60), Color::Indexed(179))
    }

    /// Streaming indicator dot.
    pub fn streaming_dot(&self) -> Color {
        self.overrides
            .get("streaming_dot")
            .copied()
            .unwrap_or_else(|| self.warning())
    }

    // -----------------------------------------------------------------------
    // Cards (Thinking, ToolCall)
    // -----------------------------------------------------------------------

    /// Thinking card accent (purple).
    pub fn think_accent(&self) -> Color {
        // RGB(167,139,250) -> index 135 (purple)
        self.color(
            "think_accent",
            Color::Rgb(167, 139, 250),
            Color::Indexed(135),
        )
    }

    /// Thinking card content text.
    pub fn think_text(&self) -> Color {
        // RGB(160,150,200) -> index 183 (light lavender)
        self.color("think_text", Color::Rgb(160, 150, 200), Color::Indexed(183))
    }

    /// Tool call card accent (cyan-blue).
    pub fn tool_card_accent(&self) -> Color {
        // RGB(0,166,255) -> index 39 (bright cyan)
        self.color(
            "tool_card_accent",
            Color::Rgb(0, 166, 255),
            Color::Indexed(39),
        )
    }

    /// Tool call card content text.
    pub fn tool_card_text(&self) -> Color {
        // RGB(140,170,200) -> index 152 (steel blue)
        self.color(
            "tool_card_text",
            Color::Rgb(140, 170, 200),
            Color::Indexed(152),
        )
    }

    // -----------------------------------------------------------------------
    // Markdown
    // -----------------------------------------------------------------------

    /// Blockquote accent / border.
    pub fn blockquote(&self) -> Color {
        // RGB(100,120,160) -> index 67 (dark steel blue)
        self.color("blockquote", Color::Rgb(100, 120, 160), Color::Indexed(67))
    }

    /// Horizontal rule color.
    pub fn hr(&self) -> Color {
        // RGB(60,60,80) -> index 241 (dark gray)
        self.color("hr", Color::Rgb(60, 60, 80), Color::Indexed(241))
    }

    /// Welcome screen accent.
    pub fn welcome(&self) -> Color {
        // RGB(100,120,180) -> index 68 (slate blue)
        self.color("welcome", Color::Rgb(100, 120, 180), Color::Indexed(68))
    }

    /// Inline code text color.
    pub fn code_fg(&self) -> Color {
        // RGB(200,220,255) -> index 189 (light blue-white)
        self.color("code_fg", Color::Rgb(200, 220, 255), Color::Indexed(189))
    }

    /// Code block content text color.
    pub fn code_block_fg(&self) -> Color {
        // RGB(180,200,220) -> index 152 (steel blue)
        self.color(
            "code_block_fg",
            Color::Rgb(180, 200, 220),
            Color::Indexed(152),
        )
    }

    // -----------------------------------------------------------------------
    // Status bar
    // -----------------------------------------------------------------------

    /// Model name text.
    pub fn status_model(&self) -> Color {
        // RGB(180,140,255) -> index 183 (light purple)
        self.color(
            "status_model",
            Color::Rgb(180, 140, 255),
            Color::Indexed(183),
        )
    }

    /// Workspace path text (teal).
    pub fn status_path(&self) -> Color {
        // RGB(0,175,175) -> index 37 (teal)
        self.color("status_path", Color::Rgb(0, 175, 175), Color::Indexed(37))
    }

    /// Context bar track (empty portion).
    pub fn status_bar_track(&self) -> Color {
        // RGB(45,45,60) -> index 239 (dark gray)
        self.color(
            "status_bar_track",
            Color::Rgb(45, 45, 60),
            Color::Indexed(239),
        )
    }

    /// Context bar fill (normal, < 80%).
    pub fn status_bar_normal(&self) -> Color {
        // RGB(100,140,255) -> index 69 (blue)
        self.color(
            "status_bar_normal",
            Color::Rgb(100, 140, 255),
            Color::Indexed(69),
        )
    }

    /// Context bar fill (warning, >= 80%).
    pub fn status_bar_warn(&self) -> Color {
        // RGB(240,192,80) -> index 179 (yellow)
        self.color(
            "status_bar_warn",
            Color::Rgb(240, 192, 80),
            Color::Indexed(179),
        )
    }

    /// Separator between status items.
    pub fn status_sep(&self) -> Color {
        self.overrides
            .get("status_sep")
            .copied()
            .unwrap_or_else(|| self.hr())
    }

    /// Cost text.
    pub fn status_cost(&self) -> Color {
        // RGB(130,130,150) -> index 245 (gray)
        self.color(
            "status_cost",
            Color::Rgb(130, 130, 150),
            Color::Indexed(245),
        )
    }

    /// Version text.
    pub fn status_version(&self) -> Color {
        // RGB(80,80,100) -> index 245 (dark gray)
        self.color(
            "status_version",
            Color::Rgb(80, 80, 100),
            Color::Indexed(245),
        )
    }

    // -----------------------------------------------------------------------
    // Input area
    // -----------------------------------------------------------------------

    /// Input area focused border.
    pub fn input_border_focused(&self) -> Color {
        // Cyan -> index 81 (bright cyan on 256-color)
        self.color("input_border_focused", Color::Cyan, Color::Indexed(81))
    }

    /// Input area unfocused border.
    pub fn input_border_unfocused(&self) -> Color {
        // DarkGray -> index 245
        self.color(
            "input_border_unfocused",
            Color::DarkGray,
            Color::Indexed(245),
        )
    }

    /// Input area title text.
    pub fn input_title(&self) -> Color {
        self.color("input_title", Color::White, Color::Indexed(255))
    }

    /// Cursor style foreground (when focused).
    pub fn cursor_fg(&self) -> Color {
        self.color("cursor_fg", Color::Black, Color::Indexed(16))
    }

    /// Cursor style background (when focused).
    pub fn cursor_bg(&self) -> Color {
        self.color("cursor_bg", Color::White, Color::Indexed(255))
    }

    /// Unfocused cursor.
    pub fn cursor_unfocused(&self) -> Color {
        self.color("cursor_unfocused", Color::DarkGray, Color::Indexed(245))
    }
}

fn valid_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn palette_overrides(
    palette: BuiltinPalette,
    truecolor: bool,
) -> Result<HashMap<String, Color>, ThemeError> {
    let semantic_values = [
        ("panel_bg", palette.background),
        ("code_bg", palette.surface),
        ("border_focused", palette.primary),
        ("title", palette.strong),
        ("text", palette.text),
        ("muted", palette.muted),
        ("user_accent", palette.user),
        ("assistant_accent", palette.primary),
        ("system_accent", palette.warning),
        ("selected", palette.primary),
        ("active", palette.success),
        ("normal", palette.text),
        ("success", palette.success),
        ("error", palette.error),
        ("warning", palette.warning),
        ("streaming_dot", palette.warning),
        ("think_accent", palette.thinking),
        ("think_text", palette.muted),
        ("tool_card_accent", palette.accent),
        ("tool_card_text", palette.text),
        ("blockquote", palette.border),
        ("hr", palette.border),
        ("welcome", palette.primary),
        ("code_fg", palette.accent),
        ("code_block_fg", palette.text),
        ("status_model", palette.thinking),
        ("status_path", palette.accent),
        ("status_bar_track", palette.border),
        ("status_bar_normal", palette.primary),
        ("status_bar_warn", palette.warning),
        ("status_sep", palette.border),
        ("status_cost", palette.muted),
        ("status_version", palette.muted),
        ("status_bar_bg", palette.surface),
        ("input_border_focused", palette.primary),
        ("input_border_unfocused", palette.border),
        ("input_title", palette.strong),
        ("cursor_fg", palette.background),
        ("cursor_bg", palette.text),
        ("cursor_unfocused", palette.muted),
    ];
    semantic_values
        .into_iter()
        .map(|(semantic, value)| {
            parse_theme_color(value, truecolor).map(|color| (semantic.to_string(), color))
        })
        .collect()
}

fn resolve_theme_color(value: ThemeColorValue, truecolor: bool) -> Result<Color, ThemeError> {
    match value {
        ThemeColorValue::Index(index) => Ok(Color::Indexed(index)),
        ThemeColorValue::Text(value) => parse_theme_color(&value, truecolor),
    }
}

fn parse_theme_color(value: &str, truecolor: bool) -> Result<Color, ThemeError> {
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(ThemeError(format!("invalid hex theme color `{value}`")));
        }
        let red = u8::from_str_radix(&hex[0..2], 16).expect("validated hex");
        let green = u8::from_str_radix(&hex[2..4], 16).expect("validated hex");
        let blue = u8::from_str_radix(&hex[4..6], 16).expect("validated hex");
        return Ok(if truecolor {
            Color::Rgb(red, green, blue)
        } else {
            Color::Indexed(rgb_to_xterm(red, green, blue))
        });
    }
    let color = match value.to_ascii_lowercase().as_str() {
        "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "dark_grey" => Color::DarkGray,
        "light_red" => Color::LightRed,
        "light_green" => Color::LightGreen,
        "light_yellow" => Color::LightYellow,
        "light_blue" => Color::LightBlue,
        "light_magenta" => Color::LightMagenta,
        "light_cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return Err(ThemeError(format!("invalid named theme color `{value}`"))),
    };
    Ok(color)
}

fn rgb_to_xterm(red: u8, green: u8, blue: u8) -> u8 {
    let cube_level = |channel: u8| -> u8 {
        if channel < 48 {
            0
        } else if channel < 115 {
            1
        } else {
            ((u16::from(channel) - 35) / 40).min(5) as u8
        }
    };
    let r = cube_level(red);
    let g = cube_level(green);
    let b = cube_level(blue);
    let cube_index = 16 + 36 * r + 6 * g + b;
    let average = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    if max.saturating_sub(min) < 10 && (8..=238).contains(&average) {
        232 + u8::try_from((average - 8) / 10).unwrap_or(23).min(23)
    } else {
        cube_index
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default_respects_env() {
        // We cannot reliably test the env var in parallel tests,
        // but we can verify the struct works.
        let theme = Theme::default();
        // Just ensure it doesn't panic and returns a valid color.
        let _ = theme.panel_bg();
        let _ = theme.border_focused();
        let _ = theme.text();
    }

    #[test]
    fn test_truecolor_returns_rgb() {
        let theme = Theme {
            truecolor: true,
            overrides: HashMap::new(),
        };
        assert!(matches!(theme.panel_bg(), Color::Rgb(_, _, _)));
        assert!(matches!(theme.border_focused(), Color::Rgb(_, _, _)));
        assert!(matches!(theme.text(), Color::Rgb(_, _, _)));
    }

    #[test]
    fn test_no_truecolor_returns_indexed() {
        let theme = Theme {
            truecolor: false,
            overrides: HashMap::new(),
        };
        assert!(matches!(theme.panel_bg(), Color::Indexed(_)));
        assert!(matches!(theme.border_focused(), Color::Indexed(_)));
        assert!(matches!(theme.text(), Color::Indexed(_)));
    }

    #[test]
    fn test_all_colors_returned() {
        let theme = Theme::default();
        // Exercise every method to ensure no panic.
        let _ = theme.panel_bg();
        let _ = theme.code_bg();
        let _ = theme.border_focused();
        let _ = theme.title();
        let _ = theme.text();
        let _ = theme.muted();
        let _ = theme.user_accent();
        let _ = theme.assistant_accent();
        let _ = theme.system_accent();
        let _ = theme.selected();
        let _ = theme.active();
        let _ = theme.normal();
        let _ = theme.success();
        let _ = theme.error();
        let _ = theme.warning();
        let _ = theme.streaming_dot();
        let _ = theme.think_accent();
        let _ = theme.think_text();
        let _ = theme.tool_card_accent();
        let _ = theme.tool_card_text();
        let _ = theme.blockquote();
        let _ = theme.hr();
        let _ = theme.welcome();
        let _ = theme.code_fg();
        let _ = theme.code_block_fg();
        let _ = theme.status_model();
        let _ = theme.status_path();
        let _ = theme.status_bar_track();
        let _ = theme.status_bar_normal();
        let _ = theme.status_bar_warn();
        let _ = theme.status_sep();
        let _ = theme.status_cost();
        let _ = theme.status_version();
        let _ = theme.input_border_focused();
        let _ = theme.input_border_unfocused();
        let _ = theme.input_title();
        let _ = theme.cursor_fg();
        let _ = theme.cursor_bg();
        let _ = theme.cursor_unfocused();
    }

    #[test]
    fn test_custom_theme_loads_semantic_color_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            themes_dir.join("ocean-dark.toml"),
            r##"
[colors]
panel_bg = "#002b36"
text = "white"
error = 160
"##,
        )
        .unwrap();

        let theme = Theme::load_for_terminal("ocean-dark", dir.path(), true).unwrap();

        assert_eq!(theme.panel_bg(), Color::Rgb(0, 43, 54));
        assert_eq!(theme.text(), Color::White);
        assert_eq!(theme.error(), Color::Indexed(160));
    }

    #[test]
    fn test_custom_theme_rejects_unknown_semantic_color() {
        let dir = tempfile::tempdir().unwrap();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            themes_dir.join("broken.toml"),
            "[colors]\nnot_a_real_token = \"#ffffff\"\n",
        )
        .unwrap();

        let error = Theme::load_for_terminal("broken", dir.path(), true).unwrap_err();

        assert!(error.to_string().contains("not_a_real_token"));
    }

    #[test]
    fn test_builtin_themes_are_available_without_a_config_directory() {
        let themes = Theme::available_themes(None);
        let names: Vec<&str> = themes.iter().map(|theme| theme.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "default",
                "dark",
                "light",
                "nord",
                "gruvbox-dark",
                "solarized-dark",
                "solarized-light",
                "dracula",
            ]
        );
        for name in names {
            Theme::load_with_capability(name, None, true)
                .unwrap_or_else(|error| panic!("built-in theme {name} did not load: {error}"));
        }
    }

    #[test]
    fn test_light_builtin_supplies_a_coherent_light_palette() {
        let theme = Theme::load_with_capability("light", None, true).unwrap();

        assert_eq!(theme.panel_bg(), Color::Rgb(250, 250, 250));
        assert_eq!(theme.text(), Color::Rgb(26, 26, 26));
        assert_eq!(theme.input_border_focused(), Color::Rgb(21, 101, 192));
        assert_eq!(theme.cursor_fg(), Color::Rgb(250, 250, 250));
        assert_eq!(theme.cursor_bg(), Color::Rgb(26, 26, 26));
    }

    #[test]
    fn test_available_themes_append_valid_custom_files_and_skip_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            themes_dir.join("ember.toml"),
            "[colors]\nwarning = \"#ff8800\"\n",
        )
        .unwrap();
        std::fs::write(
            themes_dir.join("nord.toml"),
            "[colors]\nwarning = \"#ffffff\"\n",
        )
        .unwrap();
        std::fs::write(
            themes_dir.join("broken.toml"),
            "[colors]\nnot_a_token = \"#ffffff\"\n",
        )
        .unwrap();

        let themes = Theme::available_themes(Some(dir.path()));
        let custom: Vec<&str> = themes
            .iter()
            .filter(|theme| theme.is_custom)
            .map(|theme| theme.name.as_str())
            .collect();

        assert_eq!(custom, vec!["ember"]);
        assert_eq!(
            themes.iter().filter(|theme| theme.name == "nord").count(),
            1
        );
    }
}
