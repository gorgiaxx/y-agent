//! Terminal inline-image protocol detection and encoding.
//!
//! Ports the Kitty graphics and iTerm2 inline-image protocols from
//! oh-my-pi's `terminal-capabilities.ts`, adapted for ratatui's
//! screen-buffer model: images are displayed by reserving blank rows
//! during the frame draw, then writing escape sequences directly to
//! stdout at the correct terminal coordinates after `terminal.draw()`
//! flushes the ratatui buffer.

use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::sync::{Arc, LazyLock};

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
    resolve_image_protocol(&|key| std::env::var(key).ok())
}

/// Resolve the protocol from an environment lookup.
///
/// The shipped detector and the tests share this decision table so the tested
/// behavior is the behavior that ships.
fn resolve_image_protocol(get: &dyn Fn(&str) -> Option<String>) -> Option<ImageProtocol> {
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

/// Test shim: resolve the protocol from an explicit environment table.
#[cfg(test)]
fn detect_image_protocol_from_env(env: &[(&str, String)]) -> Option<ImageProtocol> {
    resolve_image_protocol(&|key| {
        env.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| value.clone())
    })
}

/// Whether the process is running inside tmux (mirrors oh-my-pi's
/// `isInsideTmux`), so Kitty APC sequences can be wrapped in DCS
/// passthrough.
///
/// Cached: the answer cannot change for the lifetime of the process, and
/// this sits on the per-frame image path where every escape sequence is
/// wrapped.
fn is_inside_tmux() -> bool {
    static INSIDE: LazyLock<bool> = LazyLock::new(|| std::env::var_os("TMUX").is_some());
    *INSIDE
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

/// Kitty delete-all command (`d=A`): removes every image and placement
/// from the terminal's store. Called on terminal restore to clean up.
pub fn delete_all_kitty_images() -> String {
    wrap_tmux_passthrough("\x1b_Ga=d,d=A,q=2\x1b\\")
}

/// iTerm2 inline-image escape (OSC 1337). `width` is in cell units;
/// `height=auto` preserves aspect ratio.
pub fn encode_iterm2_inline(base64_data: &str, cols: u16) -> String {
    format!(
        "\x1b]1337;File=inline=1;width={cols};height=auto;preserveAspectRatio=1:{base64_data}\x07"
    )
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
/// when fit within `max_cols` wide and `max_rows` tall, preserving aspect
/// ratio. The image is scaled to the smaller of the two limits so it never
/// exceeds either dimension.
pub fn calculate_image_fit(
    dims: ImageDimensions,
    max_cols: u16,
    max_rows: u16,
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
    let max_height_px = u32::from(max_rows) * cell_h;
    let scale_w = if dims.width_px > 0 {
        f64::from(max_width_px) / f64::from(dims.width_px)
    } else {
        f64::INFINITY
    };
    let scale_h = if dims.height_px > 0 {
        f64::from(max_height_px) / f64::from(dims.height_px)
    } else {
        f64::INFINITY
    };
    // Scale to fit within both limits: use the smaller scale so the image
    // never exceeds either dimension.
    let scale = scale_w.min(scale_h);
    let fitted_width_px = (f64::from(dims.width_px) * scale) as u32;
    let fitted_height_px = (f64::from(dims.height_px) * scale) as u32;

    let cols = u16::try_from(fitted_width_px.div_ceil(cell_w))
        .unwrap_or(1)
        .max(1);
    let rows = u16::try_from(fitted_height_px.div_ceil(cell_h))
        .unwrap_or(1)
        .max(1);

    (cols.min(max_cols), rows.min(max_rows))
}

/// A pending image placement to be written to stdout after ratatui flushes.
///
/// Cloning is cheap: the base64 payload is shared with the chat render cache
/// through an [`Arc`], so assembling the per-frame placement list costs a
/// refcount bump instead of copying megabytes of text.
#[derive(Debug, Clone)]
pub struct ImagePlacement {
    /// Row of the image's top edge. Message-relative while the placement
    /// lives in the chat render cache; absolute transcript row once the chat
    /// panel offsets it for the frame.
    pub content_row: usize,
    /// Base64-encoded image data, shared with the render cache.
    pub base64_data: Arc<str>,
    /// Source pixel dimensions, needed to compute the crop rectangle when the
    /// image is only partially scrolled into view.
    pub dims: ImageDimensions,
    /// Cell columns the image occupies.
    pub cols: u16,
    /// Cell rows the image occupies when fully visible.
    pub rows: u16,
    /// Stable Kitty graphics image ID. Used for transmit-once (`a=t`) and
    /// placement (`a=p`) so the data is only sent once; subsequent frames
    /// emit a tiny placement sequence that replaces the previous one.
    pub image_id: u32,
}

/// Derive a stable Kitty image ID for one attachment occurrence.
///
/// The transcript position is folded into the hash alongside the payload:
/// the same image sent twice needs two IDs, because a placement is addressed
/// by `i`/`p` and a second `a=p` under the same pair would replace the first
/// instead of drawing a second copy.
///
/// Computed once when the message render is cached: hashing a multi-megabyte
/// base64 string on every frame dominated the scroll budget.
pub fn image_id_for(base64_data: &str, message_index: usize, ordinal: usize) -> u32 {
    let mut hasher = DefaultHasher::new();
    base64_data.hash(&mut hasher);
    message_index.hash(&mut hasher);
    ordinal.hash(&mut hasher);
    ((hasher.finish() & 0x00FF_FFFF) as u32).max(1)
}

/// The slice of an image that is visible in the viewport for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageCrop {
    /// Row of the image's first visible line, relative to the viewport top.
    pub row_in_view: u16,
    /// Display rows the visible slice occupies.
    pub visible_rows: u16,
    /// Top edge of the source rectangle, in image pixels.
    pub src_y_px: u32,
    /// Height of the source rectangle, in image pixels.
    pub src_h_px: u32,
}

/// Clip an image against the viewport `[visible_start, visible_end)`.
///
/// Returns `None` when the image lies entirely outside the viewport. The
/// source rectangle shrinks in proportion to the hidden rows, so an image
/// whose top has scrolled past the viewport reveals itself row by row instead
/// of snapping fully into view at the edge, and an image whose bottom
/// overflows is clipped instead of bleeding over the composer.
pub fn crop_to_viewport(
    placement: &ImagePlacement,
    visible_start: usize,
    visible_end: usize,
) -> Option<ImageCrop> {
    let rows = usize::from(placement.rows);
    let image_end = placement.content_row + rows;
    if rows == 0 || image_end <= visible_start || placement.content_row >= visible_end {
        return None;
    }
    let top_cut = visible_start.saturating_sub(placement.content_row);
    let bottom_cut = image_end.saturating_sub(visible_end);
    let visible_rows = rows.saturating_sub(top_cut + bottom_cut);
    if visible_rows == 0 {
        return None;
    }

    // Map the hidden row counts back onto the source image so the visible
    // slice keeps the scale of the full placement.
    let height_px = u64::from(placement.dims.height_px);
    let rows_u64 = rows as u64;
    let src_y = (height_px * top_cut as u64 / rows_u64) as u32;
    let src_end = (height_px * (rows - bottom_cut) as u64 / rows_u64) as u32;

    Some(ImageCrop {
        row_in_view: u16::try_from(placement.content_row.saturating_sub(visible_start))
            .unwrap_or(0),
        visible_rows: u16::try_from(visible_rows).unwrap_or(placement.rows),
        src_y_px: src_y,
        src_h_px: src_end.saturating_sub(src_y).max(1),
    })
}

/// Kitty transmit-only (`a=t`): sends image data keyed by `image_id` without
/// displaying it. Called once per image; the data persists in the terminal's
/// store, so subsequent frames display it with the tiny [`place_kitty`].
fn transmit_kitty(base64_data: &str, image_id: u32) -> String {
    let params = format!("a=t,f=100,q=2,i={image_id}");
    chunk_kitty_apc(&params, base64_data)
}

/// Kitty placement (`a=p`): displays a previously transmitted image at the
/// cursor position. `p={image_id}` replaces the previous placement (no
/// delete needed), so the image moves smoothly when scrolling. `C=1`
/// prevents cursor movement so our explicit cursor address stays
/// authoritative. A partially visible image adds the `y`/`h` source
/// rectangle so only its visible slice is drawn.
fn place_kitty(placement: &ImagePlacement, crop: &ImageCrop) -> String {
    let id = placement.image_id;
    let cols = placement.cols;
    let rows = crop.visible_rows;
    let mut params = format!("a=p,q=2,C=1,i={id},p={id},c={cols},r={rows}");
    if crop.visible_rows != placement.rows {
        let _ = write!(params, ",y={},h={}", crop.src_y_px, crop.src_h_px);
    }
    wrap_tmux_passthrough(&format!("\x1b_G{params}\x1b\\"))
}

/// Image IDs that have already been transmitted to the terminal.
/// Kept on `TuiApp` so the full base64 is only sent once per image.
pub type TransmittedSet = std::collections::HashSet<u32>;

/// Chat viewport geometry for one image frame.
#[derive(Debug, Clone, Copy)]
pub struct ImageViewport {
    /// First visible transcript row.
    pub scroll_to: usize,
    /// Chat area height, in rows.
    pub inner_height: usize,
    /// Chat area origin column.
    pub chat_x: u16,
    /// Chat area origin row.
    pub chat_y: u16,
}

/// Build the escape-sequence batch that draws every visible image for one
/// frame and retires the ones that scrolled out.
///
/// Kitty uses transmit-once + placement: the payload is uploaded the first
/// time an image is drawn and only a ~50 byte placement is emitted after
/// that. Images leaving the viewport are retired with `d=i`, which drops the
/// placement but *keeps* the transmitted data, so scrolling back does not
/// re-upload the payload.
///
/// iTerm2 has no source-rectangle equivalent, so its images are drawn whole
/// at the clamped row.
pub fn render_image_frame(
    protocol: ImageProtocol,
    placements: &[ImagePlacement],
    transmitted: &mut TransmittedSet,
    visible_ids: &mut TransmittedSet,
    view: ImageViewport,
) -> String {
    let visible_start = view.scroll_to;
    let visible_end = view.scroll_to + view.inner_height;

    let visible: Vec<(&ImagePlacement, ImageCrop)> = placements
        .iter()
        .filter_map(|placement| {
            crop_to_viewport(placement, visible_start, visible_end).map(|crop| (placement, crop))
        })
        .collect();
    let current_visible: TransmittedSet = visible.iter().map(|(p, _)| p.image_id).collect();

    let mut out = String::new();

    // Retire stale placements first so they never overlap the images drawn
    // below them in this same batch.
    if protocol == ImageProtocol::Kitty {
        for id in visible_ids.difference(&current_visible) {
            out.push_str(&wrap_tmux_passthrough(&format!(
                "\x1b_Ga=d,d=i,i={id},q=2\x1b\\"
            )));
        }
    }

    for (placement, crop) in visible {
        let terminal_row = view.chat_y.saturating_add(crop.row_in_view);
        let _ = write!(out, "\x1b[{};{}H", terminal_row + 1, view.chat_x + 1);
        match protocol {
            ImageProtocol::Kitty => {
                if transmitted.insert(placement.image_id) {
                    out.push_str(&transmit_kitty(&placement.base64_data, placement.image_id));
                }
                out.push_str(&place_kitty(placement, &crop));
            }
            ImageProtocol::Iterm2 => {
                out.push_str(&encode_iterm2_inline(
                    &placement.base64_data,
                    placement.cols,
                ));
            }
        }
    }

    *visible_ids = current_visible;
    out
}

/// Write the frame's image escape sequences to stdout after ratatui flushes
/// its buffer. The whole batch goes out in one write so a scroll never
/// interleaves partial graphics commands with terminal output.
pub fn emit_image_placements(
    protocol: Option<ImageProtocol>,
    placements: &[ImagePlacement],
    transmitted: &mut TransmittedSet,
    visible_ids: &mut TransmittedSet,
    view: ImageViewport,
) -> io::Result<()> {
    let Some(protocol) = protocol else {
        return Ok(());
    };
    let batch = render_image_frame(protocol, placements, transmitted, visible_ids, view);
    if batch.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(batch.as_bytes())?;
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
        assert_eq!(png_dimensions(&[0xFF; 24]), None);
    }

    #[test]
    fn calculate_image_fit_preserves_aspect_ratio() {
        let dims = ImageDimensions {
            width_px: 640,
            height_px: 480,
        };
        let (cols, rows) = calculate_image_fit(dims, 80, 30, 9, 18);
        assert_eq!(cols, 80);
        assert_eq!(rows, 30);
    }

    #[test]
    fn calculate_image_fit_caps_cols() {
        let dims = ImageDimensions {
            width_px: 1920,
            height_px: 1080,
        };
        let (cols, rows) = calculate_image_fit(dims, 40, 20, 9, 18);
        assert_eq!(cols, 40);
        assert!(rows > 0);
    }

    #[test]
    fn calculate_image_fit_caps_rows() {
        let dims = ImageDimensions {
            width_px: 640,
            height_px: 480,
        };
        let (cols, rows) = calculate_image_fit(dims, 80, 15, 9, 18);
        assert_eq!(rows, 15);
        assert!(cols <= 80);
    }

    #[test]
    fn calculate_image_fit_minimum_one_row() {
        let dims = ImageDimensions {
            width_px: 720,
            height_px: 18,
        };
        let (_, rows) = calculate_image_fit(dims, 80, 20, 9, 18);
        assert_eq!(rows, 1);
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
    fn transmit_kitty_uses_transmit_action() {
        let seq = transmit_kitty("dGVzdA==", 42);
        assert!(seq.contains("a=t"));
        assert!(seq.contains("i=42"));
        assert!(seq.contains("f=100"));
    }

    #[test]
    fn place_kitty_uses_placement_action() {
        let placement = test_placement(0, 40, 10);
        let crop = crop_to_viewport(&placement, 0, 40).expect("fully visible");
        let seq = place_kitty(&placement, &crop);
        assert!(seq.contains("a=p"));
        assert!(seq.contains(&format!("i={}", placement.image_id)));
        assert!(seq.contains(&format!("p={}", placement.image_id)));
        assert!(seq.contains("c=40"));
        assert!(seq.contains("r=10"));
        assert!(
            !seq.contains(",y=") && !seq.contains(",h="),
            "a fully visible image must not carry a source rectangle: {seq:?}"
        );
    }

    #[test]
    fn delete_all_kitty_images_uses_delete_all_action() {
        let seq = delete_all_kitty_images();
        assert!(seq.contains("a=d"));
        assert!(seq.contains("d=A"));
    }

    // --- scroll-aware cropping ---

    /// Placement with 200px height mapped onto `rows` rows, so each display
    /// row corresponds to exactly `200 / rows` source pixels.
    fn test_placement(content_row: usize, cols: u16, rows: u16) -> ImagePlacement {
        ImagePlacement {
            content_row,
            base64_data: Arc::from("dGVzdA=="),
            dims: ImageDimensions {
                width_px: 400,
                height_px: 200,
            },
            cols,
            rows,
            image_id: image_id_for("dGVzdA==", content_row, 0),
        }
    }

    fn viewport(scroll_to: usize, inner_height: usize) -> ImageViewport {
        ImageViewport {
            scroll_to,
            inner_height,
            chat_x: 0,
            chat_y: 0,
        }
    }

    #[test]
    fn crop_skips_images_outside_the_viewport() {
        let placement = test_placement(0, 40, 10);
        assert_eq!(crop_to_viewport(&placement, 10, 30), None, "fully above");
        assert_eq!(crop_to_viewport(&placement, 60, 80), None, "far above");
        let below = test_placement(100, 40, 10);
        assert_eq!(crop_to_viewport(&below, 0, 20), None, "fully below");
    }

    #[test]
    fn crop_reveals_only_the_bottom_slice_when_the_top_scrolled_off() {
        // Rows 0..10; viewport starts at row 9, so only the last image row is
        // on screen and it must sit at the very top of the viewport.
        let placement = test_placement(0, 40, 10);
        let crop = crop_to_viewport(&placement, 9, 29).expect("partially visible");
        assert_eq!(crop.row_in_view, 0);
        assert_eq!(crop.visible_rows, 1);
        assert_eq!(crop.src_y_px, 180);
        assert_eq!(crop.src_h_px, 20);
    }

    #[test]
    fn crop_clips_the_overflowing_bottom_of_an_image() {
        // Rows 5..15 against a 10-row viewport: the last 5 rows overflow and
        // must be clipped instead of drawn over the composer.
        let placement = test_placement(5, 40, 10);
        let crop = crop_to_viewport(&placement, 0, 10).expect("partially visible");
        assert_eq!(crop.row_in_view, 5);
        assert_eq!(crop.visible_rows, 5);
        assert_eq!(crop.src_y_px, 0);
        assert_eq!(crop.src_h_px, 100);
    }

    #[test]
    fn crop_handles_an_image_taller_than_the_viewport() {
        let placement = test_placement(0, 40, 10);
        let crop = crop_to_viewport(&placement, 3, 7).expect("middle band visible");
        assert_eq!(crop.row_in_view, 0);
        assert_eq!(crop.visible_rows, 4);
        assert_eq!(crop.src_y_px, 60);
        assert_eq!(crop.src_h_px, 80);
    }

    #[test]
    fn frame_emits_source_rectangle_for_a_partially_scrolled_image() {
        let placements = vec![test_placement(0, 40, 10)];
        let mut transmitted = TransmittedSet::new();
        let mut visible = TransmittedSet::new();
        let seq = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(9, 20),
        );
        assert!(seq.contains("r=1"), "only one row is visible: {seq:?}");
        assert!(
            seq.contains("y=180"),
            "crop must start below the top: {seq:?}"
        );
        assert!(seq.contains("h=20"), "crop must be one row tall: {seq:?}");
    }

    #[test]
    fn frame_transmits_payload_once_and_keeps_it_across_scrolls() {
        let placements = vec![test_placement(0, 40, 10)];
        let mut transmitted = TransmittedSet::new();
        let mut visible = TransmittedSet::new();

        let first = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(0, 20),
        );
        assert!(first.contains("a=t"), "first frame uploads the payload");

        // Scroll the image out of view: the placement is retired but the
        // payload must stay in the terminal's store (`d=i`, not `d=I`).
        let gone = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(50, 20),
        );
        assert!(gone.contains("d=i"), "placement retired: {gone:?}");
        assert!(!gone.contains("d=I"), "payload must survive: {gone:?}");
        assert!(visible.is_empty());

        // Scroll back: no re-upload, only a placement.
        let back = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(0, 20),
        );
        assert!(
            !back.contains("a=t"),
            "payload must not be re-sent: {back:?}"
        );
        assert!(back.contains("a=p"));
    }

    #[test]
    fn frame_positions_images_at_the_chat_origin() {
        let placements = vec![test_placement(2, 40, 4)];
        let mut transmitted = TransmittedSet::new();
        let mut visible = TransmittedSet::new();
        let seq = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            ImageViewport {
                scroll_to: 0,
                inner_height: 20,
                chat_x: 3,
                chat_y: 5,
            },
        );
        // Row 2 of the transcript with the chat area starting at row 5 lands
        // on 1-based terminal row 8, column 4.
        assert!(seq.contains("\x1b[8;4H"), "cursor address: {seq:?}");
    }

    #[test]
    fn frame_is_empty_when_nothing_is_visible() {
        let placements = vec![test_placement(100, 40, 4)];
        let mut transmitted = TransmittedSet::new();
        let mut visible = TransmittedSet::new();
        let seq = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(0, 20),
        );
        assert!(seq.is_empty());
        assert!(transmitted.is_empty(), "never uploaded off-screen images");
    }

    #[test]
    fn identical_payloads_at_different_positions_get_distinct_ids() {
        // The same screenshot sent twice must draw twice: a shared `i`/`p`
        // pair would make the second placement replace the first.
        let first = image_id_for("dGVzdA==", 3, 0);
        let second = image_id_for("dGVzdA==", 7, 0);
        let sibling = image_id_for("dGVzdA==", 3, 1);
        assert_ne!(first, second);
        assert_ne!(first, sibling);
        assert_eq!(first, image_id_for("dGVzdA==", 3, 0), "ids are stable");

        let placements = vec![test_placement(0, 40, 4), test_placement(6, 40, 4)];
        let mut transmitted = TransmittedSet::new();
        let mut visible = TransmittedSet::new();
        let seq = render_image_frame(
            ImageProtocol::Kitty,
            &placements,
            &mut transmitted,
            &mut visible,
            viewport(0, 20),
        );
        assert_eq!(visible.len(), 2, "both copies are placed: {seq:?}");
    }
}
