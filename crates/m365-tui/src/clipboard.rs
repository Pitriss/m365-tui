//! Copying text to the system clipboard.
//!
//! Prefers a native helper (`wl-copy`, `xclip`, `xsel`) when one is on `PATH`,
//! since those work regardless of terminal support. Otherwise falls back to the
//! OSC 52 escape sequence, which most modern terminals honour and which also
//! works over SSH.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Copy `text` to the clipboard. Returns the mechanism used, for the status line.
pub fn copy(text: &str) -> Result<&'static str> {
    if let Some(tool) = via_helper(text) {
        return Ok(tool);
    }
    via_osc52(text)?;
    Ok("OSC 52")
}

fn via_helper(text: &str) -> Option<&'static str> {
    const CANDIDATES: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];

    for (bin, args) in CANDIDATES {
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue; // not installed
        };
        // Write then drop stdin so the helper sees EOF.
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(text.as_bytes()).is_err() {
                let _ = child.kill();
                continue;
            }
        }
        // wl-copy daemonizes to hold the selection, so this returns promptly.
        let _ = child.wait();
        return Some(bin);
    }
    None
}

/// Terminal-native clipboard write. Safe to emit while in raw mode.
fn via_osc52(text: &str) -> Result<()> {
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))
        .context("writing OSC 52 sequence")?;
    out.flush().context("flushing OSC 52 sequence")?;
    Ok(())
}

/// Standard base64 with padding (OSC 52 payloads must be base64-encoded).
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // multi-byte UTF-8 round-trips through the byte encoder
        assert_eq!(base64("é".as_bytes()), "w6k=");
    }
}
