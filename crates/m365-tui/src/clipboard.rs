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
    write!(
        out,
        "\x1b]52;c;{}\x07",
        m365_core::util::base64_encode(text.as_bytes())
    )
        .context("writing OSC 52 sequence")?;
    out.flush().context("flushing OSC 52 sequence")?;
    Ok(())
}
