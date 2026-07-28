use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;

const OSC52_MAX_INPUT_BYTES: usize = 100_000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Prefix of fallback clipboard files written to the temp dir.
const FALLBACK_FILE_PREFIX: &str = "y-agent-copy-";

/// Fallback files older than this are deleted when a new one is written.
const FALLBACK_FILE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRoute {
    Native,
    Osc52,
    TmuxOsc52,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardDelivery {
    Native,
    Osc52,
    FallbackFile(PathBuf),
}

/// PNG-encoded clipboard image ready for the typed attachment contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub png_data: Vec<u8>,
}

/// Read image media from the native clipboard and normalize it to PNG.
pub fn read_image() -> Result<ClipboardImage, String> {
    let image = arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_image())
        .map_err(|error| error.to_string())?;
    if image.bytes.len() > MAX_IMAGE_BYTES.saturating_mul(4) {
        return Err("clipboard image exceeds the 20 MB attachment limit".into());
    }
    let png_data = encode_rgba_png(image.width, image.height, image.bytes.as_ref())?;
    if png_data.len() > MAX_IMAGE_BYTES {
        return Err("encoded clipboard image exceeds the 20 MB attachment limit".into());
    }
    Ok(ClipboardImage {
        width: image.width,
        height: image.height,
        png_data,
    })
}

fn encode_rgba_png(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "clipboard image dimensions overflow RGBA size".to_string())?;
    if expected != rgba.len() {
        return Err(format!(
            "clipboard RGBA length mismatch: expected {expected}, received {}",
            rgba.len()
        ));
    }
    let width = u32::try_from(width).map_err(|_| "clipboard image width is too large")?;
    let height = u32::try_from(height).map_err(|_| "clipboard image height is too large")?;
    let mut encoded = Vec::new();
    {
        let mut png_encoder = png::Encoder::new(&mut encoded, width, height);
        png_encoder.set_color(png::ColorType::Rgba);
        png_encoder.set_depth(png::BitDepth::Eight);
        let mut writer = png_encoder
            .write_header()
            .map_err(|error| error.to_string())?;
        writer
            .write_image_data(rgba)
            .map_err(|error| error.to_string())?;
    }
    Ok(encoded)
}

pub fn copy_text(
    text: &str,
    capabilities: crate::tui::terminal::TerminalCapabilities,
) -> Result<ClipboardDelivery, String> {
    let tmux = matches!(
        capabilities.host,
        crate::tui::terminal::TerminalHost::Tmux | crate::tui::terminal::TerminalHost::TmuxOverSsh
    );
    let route = choose_clipboard_route(capabilities.supports_osc52_copy(), tmux);

    let primary = match route {
        ClipboardRoute::Native => native_copy(text).map(|()| ClipboardDelivery::Native),
        ClipboardRoute::Osc52 => write_osc52(text, false).map(|()| ClipboardDelivery::Osc52),
        ClipboardRoute::TmuxOsc52 => write_osc52(text, true).map(|()| ClipboardDelivery::Osc52),
    };

    primary.or_else(|_| {
        write_fallback_file(text)
            .map(ClipboardDelivery::FallbackFile)
            .map_err(|error| format!("clipboard and fallback file failed: {error}"))
    })
}

fn choose_clipboard_route(remote: bool, tmux: bool) -> ClipboardRoute {
    if tmux {
        ClipboardRoute::TmuxOsc52
    } else if remote {
        ClipboardRoute::Osc52
    } else {
        ClipboardRoute::Native
    }
}

fn native_copy(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| error.to_string())
}

fn write_osc52(text: &str, tmux: bool) -> Result<(), String> {
    let sequence = osc52_sequence(text, tmux)?;
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(sequence.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| error.to_string())
}

fn osc52_sequence(text: &str, tmux: bool) -> Result<String, String> {
    if text.len() > OSC52_MAX_INPUT_BYTES {
        return Err(format!(
            "content exceeds OSC52 limit of {OSC52_MAX_INPUT_BYTES} bytes"
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let sequence = format!("\u{1b}]52;c;{encoded}\u{7}");
    if tmux {
        Ok(format!("\u{1b}Ptmux;\u{1b}{sequence}\u{1b}\\"))
    } else {
        Ok(sequence)
    }
}

fn write_fallback_file(text: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir();
    cleanup_stale_fallback_files(&dir);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!(
        "{FALLBACK_FILE_PREFIX}{}-{timestamp}.txt",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(text.as_bytes())?;
    Ok(path)
}

/// Delete fallback clipboard files older than `FALLBACK_FILE_MAX_AGE` in `dir`.
///
/// Only files matching the exact `y-agent-copy-*.txt` pattern are touched.
/// Cleanup is best-effort: every error is ignored so it can never break a
/// copy operation.
fn cleanup_stale_fallback_files(dir: &Path) {
    let now = SystemTime::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with(FALLBACK_FILE_PREFIX) {
            continue;
        }
        let is_txt = Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
        if !is_txt {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .is_some_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age > FALLBACK_FILE_MAX_AGE)
            });
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_route_prefers_tmux_osc52_for_remote_tmux() {
        assert_eq!(
            choose_clipboard_route(true, true),
            ClipboardRoute::TmuxOsc52
        );
    }

    #[test]
    fn test_clipboard_route_uses_native_for_local_terminal() {
        assert_eq!(choose_clipboard_route(false, false), ClipboardRoute::Native);
    }

    #[test]
    fn test_osc52_sequence_contains_encoded_payload() {
        let sequence = osc52_sequence("hello", false).unwrap();
        assert!(sequence.contains("aGVsbG8="));
        assert!(sequence.starts_with("\u{1b}]52;c;"));
    }

    // T-CLIPBOARD-CLEANUP-01: stale fallback files are removed, fresh and
    // non-matching files are left untouched.
    #[test]
    fn test_cleanup_removes_only_stale_prefixed_files() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("y-agent-copy-1-1000.txt");
        let fresh = dir.path().join("y-agent-copy-1-2000.txt");
        let unrelated = dir.path().join("other-app-1-1000.txt");
        let wrong_suffix = dir.path().join("y-agent-copy-1-1000.log");
        for path in [&stale, &fresh, &unrelated, &wrong_suffix] {
            std::fs::write(path, "payload").unwrap();
        }

        // Age the stale and unrelated files beyond the 24h cutoff.
        let old = std::fs::FileTimes::new()
            .set_modified(SystemTime::now() - FALLBACK_FILE_MAX_AGE - Duration::from_secs(60));
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_times(old)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&unrelated)
            .unwrap()
            .set_times(old)
            .unwrap();

        cleanup_stale_fallback_files(dir.path());

        assert!(!stale.exists(), "stale fallback file should be removed");
        assert!(fresh.exists(), "fresh fallback file should be kept");
        assert!(unrelated.exists(), "unrelated file should be kept");
        assert!(wrong_suffix.exists(), "non-.txt file should be kept");
    }

    // T-CLIPBOARD-CLEANUP-02: cleanup tolerates a missing directory.
    #[test]
    fn test_cleanup_ignores_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        cleanup_stale_fallback_files(&missing); // Must not panic.
    }

    #[test]
    fn test_rgba_clipboard_pixels_encode_as_png() {
        let encoded = encode_rgba_png(1, 1, &[255, 0, 0, 255]).unwrap();

        assert!(encoded.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_rgba_encoder_rejects_inconsistent_dimensions() {
        let error = encode_rgba_png(2, 2, &[255, 0, 0, 255]).unwrap_err();

        assert!(error.contains("RGBA"));
    }
}
