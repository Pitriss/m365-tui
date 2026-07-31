//! Opening links in the user's browser.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Open `url` with the system handler. Spawns detached so the TUI never blocks
/// and the browser's own output can't scribble on the terminal.
pub fn open_url(url: &str) -> Result<()> {
    // Only hand off things that look like links, never arbitrary shell input.
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        anyhow::bail!("refusing to open non-http(s) link");
    }

    const CANDIDATES: &[&str] = &["xdg-open", "open"];
    for bin in CANDIDATES {
        let spawned = Command::new(bin)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("no opener found (tried xdg-open, open)"))
        .context("could not launch a browser")
}

#[cfg(test)]
mod tests {
    use super::open_url;

    #[test]
    fn rejects_non_web_schemes() {
        // Guards against a crafted href turning into a local command.
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("; rm -rf /").is_err());
    }
}
