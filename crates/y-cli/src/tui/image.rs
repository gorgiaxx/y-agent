//! Terminal inline-image protocol detection and encoding.
//!
//! Ports the Kitty graphics and iTerm2 inline-image protocols from
//! oh-my-pi's `terminal-capabilities.ts`, adapted for ratatui's
//! screen-buffer model: images are displayed by reserving blank rows
//! during the frame draw, then writing escape sequences directly to
//! stdout at the correct terminal coordinates after `terminal.draw()`
//! flushes the ratatui buffer.

use std::io::{self, Write};

/// Which inline-image protocol the terminal supports, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    /// Kitty graphics protocol (APC `_G..._\`).
    Kitty,
    /// iTerm2 inline-image protocol (OSC 1337).
    Iterm2,
}

/// Detect the terminal's inline-image protocol from environment markers.
///
/// Mirrors oh-my-pi's `detectTerminalId` + `KNOWN_TERMINALS` image-protocol
/// resolution: kitty, ghostty, wezterm, and warp (macOS/Linux) use the Kitty
/// graphics protocol; iTerm2 uses its own inline-image protocol. Inside tmux
/// or screen, the Kitty protocol is forwarded via DCS passthrough when
/// `allow-passthrough` is enabled, so it is still selected as a fallback.
pub fn detect_image_protocol() -> Option<ImageProtocol> {
    fn get(key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    if get("KITTY_WINDOW_ID").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("GHOSTTY_RESOURCES_DIR").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("WEZTERM_PANE").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("ITERM_SESSION_ID").is_some() {
        return Some(ImageProtocol::Iterm2);
    }

    if let Some(tp) = get("TERM_PROGRAM") {
        let tp_lower = tp.to_lowercase();
        match tp_lower.as_str() {
            "kitty" | "ghostty" | "wezterm" => return Some(ImageProtocol::Kitty),
            "iterm.app" => return Some(ImageProtocol::Iterm2),
            "warpterminal" => {
                let is_windows = get("WSL_DISTRO_NAME").is_some() || get("WSL_INTEROP").is_some();
                if !is_windows {
                    return Some(ImageProtocol::Kitty);
                }
            }
            _ => {}
        }
    }

    if let Some(term) = get("TERM") {
        let term_lower = term.to_lowercase();
        if term_lower.contains("ghostty")
            || term_lower.contains("tmux")
            || term_lower.contains("screen")
        {
            return Some(ImageProtocol::Kitty);
        }
    }

    None
}
#[cfg(test)]

fn detect_image_protocol_from_env(env: &[(&str, String)]) -> Option<ImageProtocol> {
    let get = |key: &str| -> Option<&str> {
        env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    };

    if get("KITTY_WINDOW_ID").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("GHOSTTY_RESOURCES_DIR").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("WEZTERM_PANE").is_some() {
        return Some(ImageProtocol::Kitty);
    }
    if get("ITERM_SESSION_ID").is_some() {
        return Some(ImageProtocol::Iterm2);
    }

    if let Some(tp) = get("TERM_PROGRAM") {
        let tp_lower = tp.to_lowercase();
        match tp_lower.as_str() {
            "kitty" | "ghostty" | "wezterm" => return Some(ImageProtocol::Kitty),
            "iterm.app" => return Some(ImageProtocol::Iterm2),
            "warpterminal" => {
                let is_windows = get("WSL_DISTRO_NAME").is_some() || get("WSL_INTEROP").is_some();
                if !is_windows {
                    return Some(ImageProtocol::Kitty);
                }
            }
            _ => {}
        }
    }

    if let Some(term) = get("TERM") {
        let term_lower = term.to_lowercase();
        if term_lower.contains("ghostty")
            || term_lower.contains("tmux")
            || term_lower.contains("screen")
        {
            return Some(ImageProtocol::Kitty);
        }
    }

    None
}

/// Whether the process is running inside tmux (mirrors oh-my-pi's
/// `isInsideTmux`), so Kitty APC sequences can be wrapped in DCS
/// passthrough.
fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

/// Wrap a raw escape sequence in tmux's DCS passthrough envelope when
/// inside tmux. The outer terminal receives the sequence verbatim only if
/// `allow-passthrough on` is set in tmux.conf.
fn wrap_tmux_passthrough(raw: &str) -> String {
    if is_inside_tmux() {
        format!("\x1bPtmux;\x1b{raw}\x1b\\")
    } else {
        raw.to_string()
    }
}

/// Chunk a Kitty APC so the base64 payload fits under terminal
/// line-buffer limits. Each chunk is a complete `_G..._\` escape;
/// `m=1` marks continuation, `m=0` marks the final chunk.
fn chunk_kitty_apc(lead_params: &str, base64_data: &str) -> String {
    const CHUNK_SIZE: usize = 4096;

    if base64_data.len() <= CHUNK_SIZE {
        return wrap_tmux_passthrough(&format!("\x1b_G{lead_params};{base64_data}\x1b\\"));
    }

    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut is_first = true;

    while offset < base64_data.len() {
        let end = (offset + CHUNK_SIZE).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let is_last = end >= base64_data.len();

        let params = if is_first {
            format!("{lead_params},m=1")
        } else if is_last {
            "q=2,m=0".to_string()
        } else {
            "q=2,m=1".to_string()
        };

        chunks.push(wrap_tmux_passthrough(&format!(
            "\x1b_G{params};{chunk}\x1b\\"
        )));

        offset = end;
        is_first = false;
    }

    chunks.join("")
}

/// Kitty transmit-and-display (`a=T`): self-contained form that sends the
/// image data and displays it in one sequence.
///
/// `C=1` keeps the terminal cursor anchored at the placement origin.
/// `f=100` selects PNG format. `q=2` suppresses the terminal reply.
pub fn encode_kitty_inline(base64_data: &str, cols: u16, rows: u16) -> String {
    let params = format!("a=T,f=100,q=2,C=1,c={cols},r={rows}");
    chunk_kitty_apc(&params, base64_data)
}

/// iTerm2 inline-image escape (OSC 1337). `width` is in cell units;
/// `height=auto` preserves aspect ratio.
pub fn encode_iterm2_inline(base64_data: &str, cols: u16) -> String {
    format!(
        "\x1b]1337;File=inline=1;width={cols};height=auto;preserveAspectRatio=1:{base64_data}\x07"
    )
}

/// Encode an image for display using the detected protocol.
/// Returns `None` when the protocol is `None` (no inline-image support).
pub fn encode_image(
    protocol: Option<ImageProtocol>,
    base64_data: &str,
    cols: u16,
    rows: u16,
) -> Option<String> {
    match protocol? {
        ImageProtocol::Kitty => Some(encode_kitty_inline(base64_data, cols, rows)),
        ImageProtocol::Iterm2 => Some(encode_iterm2_inline(base64_data, cols)),
    }
}

// --- PNG dimension parsing ---

/// Pixel dimensions of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

/// Parse PNG width and height from the IHDR chunk.
///
/// PNG layout: 8-byte signature, then IHDR chunk (4-byte length, "IHDR",
/// 4-byte width, 4-byte height). Width is at byte offset 16, height at 20.
pub fn png_dimensions(data: &[u8]) -> Option<ImageDimensions> {
    if data.len() < 24 {
        return None;
    }
    if data[0..8] != [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    if width == 0 || height == 0 {
        return None;
    }
    Some(ImageDimensions {
        width_px: width,
        height_px: height,
    })
}

// --- Image fit calculation ---

const DEFAULT_CELL_WIDTH_PX: u32 = 9;
const DEFAULT_CELL_HEIGHT_PX: u32 = 18;

/// Compute the number of terminal cell columns and rows an image occupies
/// when fit to `max_cols` wide, preserving aspect ratio.
pub fn calculate_image_fit(
    dims: ImageDimensions,
    max_cols: u16,
    cell_width_px: u32,
    cell_height_px: u32,
) -> (u16, u16) {
    let cell_w = if cell_width_px > 0 {
        cell_width_px
    } else {
        DEFAULT_CELL_WIDTH_PX
    };
    let cell_h = if cell_height_px > 0 {
        cell_height_px
    } else {
        DEFAULT_CELL_HEIGHT_PX
    };

    let max_width_px = u32::from(max_cols) * cell_w;
    let scale = if dims.width_px > 0 {
        f64::from(max_width_px) / f64::from(dims.width_px)
    } else {
        1.0
    };
    let fitted_width_px = (f64::from(dims.width_px) * scale) as u32;
    let fitted_height_px = (f64::from(dims.height_px) * scale) as u32;

    let cols = u16::try_from(fitted_width_px.div_ceil(cell_w))
        .unwrap_or(1)
        .max(1);
    let rows = u16::try_from(fitted_height_px.div_ceil(cell_h))
        .unwrap_or(1)
        .max(1);

    (cols.min(max_cols), rows)
}

// --- Post-render emission ---

/// A pending image placement to be written to stdout after ratatui flushes.
#[derive(Debug, Clone)]
pub struct ImagePlacement {
    /// Absolute content-space row (0 = top of transcript).
    pub content_row: usize,
    /// Base64-encoded image data.
    pub base64_data: String,
    /// Cell columns the image occupies.
    pub cols: u16,
    /// Cell rows the image occupies.
    pub rows: u16,
}

/// Write image escape sequences directly to stdout at the given terminal
/// positions. Called after `terminal.draw()` flushes the ratatui frame.
///
/// Only images whose content row falls in the visible window are emitted.
#[allow(clippy::too_many_arguments)]
pub fn emit_image_placements(
    protocol: Option<ImageProtocol>,
    placements: &[ImagePlacement],
    scroll_to: usize,
    inner_height: usize,
    chat_x: u16,
    chat_y: u16,
) -> io::Result<()> {
    let Some(protocol) = protocol else {
        return Ok(());
    };

    let mut stdout = io::stdout().lock();
    for placement in placements {
        let visible_start = scroll_to;
        let visible_end = scroll_to + inner_height;
        if placement.content_row < visible_start || placement.content_row >= visible_end {
            continue;
        }
        let terminal_row =
            chat_y + u16::try_from(placement.content_row - visible_start).unwrap_or(0);
        let move_to = format!("\x1b[{terminal_row};{chat_x}H");
        let Some(seq) = encode_image(
            Some(protocol),
            &placement.base64_data,
            placement.cols,
            placement.rows,
        ) else {
            continue;
        };
        stdout.write_all(move_to.as_bytes())?;
        stdout.write_all(seq.as_bytes())?;
    }
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_kitty_from_window_id() {
        let env = vec![("KITTY_WINDOW_ID", "1".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_kitty_from_term_program() {
        let env = vec![("TERM_PROGRAM", "kitty".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_iterm2_from_session_id() {
        let env = vec![("ITERM_SESSION_ID", "session-1".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Iterm2)
        );
    }

    #[test]
    fn detect_iterm2_from_term_program() {
        let env = vec![("TERM_PROGRAM", "iTerm.app".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Iterm2)
        );
    }

    #[test]
    fn detect_ghostty_from_resources_dir() {
        let env = vec![("GHOSTTY_RESOURCES_DIR", "/path".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_wezterm_from_pane() {
        let env = vec![("WEZTERM_PANE", "1".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_none_for_unsupported() {
        let env = vec![("TERM_PROGRAM", "vscode".to_string())];
        assert_eq!(detect_image_protocol_from_env(&env), None);
    }

    #[test]
    fn detect_kitty_from_tmux_term() {
        let env = vec![("TERM", "tmux-256color".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_warp_on_macos() {
        let env = vec![("TERM_PROGRAM", "WarpTerminal".to_string())];
        assert_eq!(
            detect_image_protocol_from_env(&env),
            Some(ImageProtocol::Kitty)
        );
    }

    #[test]
    fn detect_warp_disabled_on_wsl() {
        let env = vec![
            ("TERM_PROGRAM", "WarpTerminal".to_string()),
            ("WSL_DISTRO_NAME", "Ubuntu".to_string()),
        ];
        assert_eq!(detect_image_protocol_from_env(&env), None);
    }

    #[test]
    fn png_dimensions_parses_valid_png() {
        let mut data = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52,
        ];
        data.extend_from_slice(&[0x00, 0x00, 0x02, 0x80]); // width = 640
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]); // height = 480
        data.extend_from_slice(&[0x08, 0x02, 0x00, 0x00, 0x00]);
        assert_eq!(
            png_dimensions(&data),
            Some(ImageDimensions {
                width_px: 640,
                height_px: 480
            })
        );
    }

    #[test]
    fn png_dimensions_rejects_short_data() {
        assert_eq!(png_dimensions(&[0x89, 0x50]), None);
    }

    #[test]
    fn png_dimensions_rejects_bad_signature() {
        assert_eq!(png_dimensions(&vec![0xFF; 24]), None);
    }

    #[test]
    fn calculate_image_fit_preserves_aspect_ratio() {
        let dims = ImageDimensions {
            width_px: 640,
            height_px: 480,
        };
        let (cols, rows) = calculate_image_fit(dims, 80, 9, 18);
        assert_eq!(cols, 80);
        assert_eq!(rows, 30);
    }

    #[test]
    fn calculate_image_fit_caps_cols() {
        let dims = ImageDimensions {
            width_px: 1920,
            height_px: 1080,
        };
        let (cols, rows) = calculate_image_fit(dims, 40, 9, 18);
        assert_eq!(cols, 40);
        assert!(rows > 0);
    }

    #[test]
    fn calculate_image_fit_minimum_one_row() {
        let dims = ImageDimensions {
            width_px: 720,
            height_px: 18,
        };
        let (_, rows) = calculate_image_fit(dims, 80, 9, 18);
        assert_eq!(rows, 1);
    }

    #[test]
    fn encode_kitty_inline_contains_transmit_and_display() {
        let seq = encode_kitty_inline("dGVzdA==", 40, 10);
        assert!(seq.contains("a=T"));
        assert!(seq.contains("c=40"));
        assert!(seq.contains("r=10"));
        assert!(seq.contains("f=100"));
    }

    #[test]
    fn encode_kitty_inline_chunks_large_data() {
        let large_data = "A".repeat(5000);
        let seq = encode_kitty_inline(&large_data, 40, 10);
        let count = seq.matches("\x1b_G").count();
        assert!(
            count >= 2,
            "expected chunked output, got {count} APC sequences"
        );
        assert!(seq.contains("m=1"));
        assert!(seq.contains("m=0"));
    }

    #[test]
    fn encode_iterm2_inline_contains_file_command() {
        let seq = encode_iterm2_inline("dGVzdA==", 40);
        assert!(seq.contains("1337;File="));
        assert!(seq.contains("inline=1"));
        assert!(seq.contains("width=40"));
        assert!(seq.contains("height=auto"));
    }

    #[test]
    fn encode_image_returns_none_for_no_protocol() {
        assert!(encode_image(None, "dGVzdA==", 40, 10).is_none());
    }

    #[test]
    fn encode_image_returns_sequence_for_kitty() {
        let seq = encode_image(Some(ImageProtocol::Kitty), "dGVzdA==", 40, 10);
        assert!(seq.is_some());
        assert!(seq.unwrap().contains("a=T"));
    }

    #[test]
    fn encode_image_returns_sequence_for_iterm2() {
        let seq = encode_image(Some(ImageProtocol::Iterm2), "dGVzdA==", 40, 10);
        assert!(seq.is_some());
        assert!(seq.unwrap().contains("1337;File="));
    }
}
