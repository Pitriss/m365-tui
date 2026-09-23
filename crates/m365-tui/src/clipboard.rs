//! Copying text to the system clipboard.
//!
//! Prefers a native helper (`wl-copy`, `xclip`, `xsel`, `pbcopy`) when one is on
//! `PATH`, since those work regardless of terminal support. General message
//! copying falls back to OSC 52. The diagnostics exporter intentionally uses
//! only a native helper and falls back to a private log file instead.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

const CANDIDATES: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
    ("pbcopy", &[]),
];

/// Copy `text` to the clipboard. Returns the mechanism used, for the status line.
pub fn copy(text: &str) -> Result<&'static str> {
    if let Some(tool) = copy_native(text) {
        return Ok(tool);
    }
    via_osc52(text)?;
    Ok("OSC 52")
}

/// Try only native clipboard helpers.
///
/// Diagnostics use this so a machine without a clipboard program gets a real
/// log file instead of an OSC 52 sequence that might be silently ignored.
pub fn copy_native(text: &str) -> Option<&'static str> {
    for (bin, args) in CANDIDATES {
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };

        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        if !wrote {
            let _ = child.kill();
            let _ = child.wait();
            continue;
        }

        if child.wait().is_ok_and(|status| status.success()) {
            return Some(bin);
        }
    }
    None
}

/// Return the first native helper visible on PATH without executing it.
pub fn native_backend() -> Option<&'static str> {
    let path = std::env::var_os("PATH")?;
    CANDIDATES.iter().find_map(|(bin, _)| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(bin))
            .any(|candidate| candidate.is_file())
            .then_some(*bin)
    })
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
