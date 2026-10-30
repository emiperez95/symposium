//! Locating the host tools by absolute path, and reading the git state.
//!
//! Nothing here goes through asdf shims or `$PATH` lookups inside the fake
//! HOME, where shims break.

use anyhow::{Context, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Tools {
    pub srt: PathBuf,
    pub node_bin: PathBuf,
    pub tmux: PathBuf,
    pub toolchain_bin: PathBuf,
    pub real_home: PathBuf,
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub fn output(cmd: &mut Command) -> anyhow::Result<String> {
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("{:?} exited with {}", cmd.get_program(), out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Tools {
    pub fn discover() -> anyhow::Result<Tools> {
        Ok(Tools {
            srt: find_srt()?,
            node_bin: find_node_bin()?,
            tmux: which("tmux")
                .ok_or_else(|| anyhow!("tmux not found on PATH (brew install tmux)"))?,
            toolchain_bin: toolchain_bin()?,
            real_home: real_home()?,
        })
    }
}

/// The real `srt` script, not the asdf shim.
pub fn find_srt() -> anyhow::Result<PathBuf> {
    let prefix = output(Command::new("npm").args(["prefix", "-g"]))
        .context("`npm prefix -g` failed; is node installed?")?;
    let srt = Path::new(&prefix).join("bin/srt");
    if !srt.is_file() {
        bail!(
            "srt not found at {}; install it with `npm i -g @anthropic-ai/sandbox-runtime`",
            srt.display()
        );
    }
    Ok(srt)
}

pub fn find_node_bin() -> anyhow::Result<PathBuf> {
    let node =
        output(Command::new("sh").args(["-c", "asdf which node 2>/dev/null || command -v node"]))
            .context("node not found")?;
    let real = std::fs::canonicalize(&node).unwrap_or_else(|_| PathBuf::from(&node));
    real.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("cannot locate node's directory"))
}

pub fn toolchain_bin() -> anyhow::Result<PathBuf> {
    let cargo = output(Command::new("rustup").args(["which", "cargo"]))
        .context("`rustup which cargo` failed")?;
    Path::new(&cargo)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("cannot locate the toolchain bin directory"))
}

/// The real HOME from the user database, not `$HOME` (which may be redirected).
pub fn real_home() -> anyhow::Result<PathBuf> {
    let user = std::env::var("USER").context("USER is not set")?;
    let out = output(Command::new("dscl").args([
        ".",
        "-read",
        &format!("/Users/{user}"),
        "NFSHomeDirectory",
    ]));
    if let Ok(out) = out
        && let Some(h) = out.split_whitespace().last()
    {
        return Ok(PathBuf::from(h));
    }
    // Not macOS: the passwd database.
    let out = output(Command::new("getent").args(["passwd", &user]))
        .context("cannot determine the real home directory")?;
    out.split(':')
        .nth(5)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("malformed passwd entry"))
}

/// `sandbox-exec` succeeds only when we are not already inside a macOS sandbox.
pub fn nested_sandbox() -> bool {
    Command::new("sandbox-exec")
        .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(false)
}

/// `<short commit>[-dirty]` for the checkout holding `dir`.
pub fn symposium_commit(dir: &Path) -> String {
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(dir).args(args).output();
    let Ok(rev) = git(&["rev-parse", "--short", "HEAD"]) else {
        return "unknown".into();
    };
    if !rev.status.success() {
        return "unknown".into();
    }
    let commit = String::from_utf8_lossy(&rev.stdout).trim().to_string();
    let clean = git(&["diff", "--quiet", "--", "src", "symposium-sdk"])
        .map(|o| o.status.success())
        .unwrap_or(true);
    if clean {
        commit
    } else {
        format!("{commit}-dirty")
    }
}

/// Quote for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
