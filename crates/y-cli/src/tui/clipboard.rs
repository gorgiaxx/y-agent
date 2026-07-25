use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;

const OSC52_MAX_INPUT_BYTES: usize = 100_000;

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

pub fn copy_text(text: &str) -> Result<ClipboardDelivery, String> {
    let remote = std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_CLIENT").is_some()
        || std::env::var_os("SSH_TTY").is_some();
    let tmux = std::env::var_os("TMUX").is_some();
    let route = choose_clipboard_route(remote, tmux);

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
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = std::env::temp_dir().join(format!(
        "y-agent-copy-{}-{timestamp}.txt",
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
}
