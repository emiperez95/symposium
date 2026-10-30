//! `sandbox check`: preflight.

use std::path::Path;

use super::agent::Agent;
use super::tools;

pub struct Item {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

fn item(name: &str, r: anyhow::Result<String>) -> Item {
    match r {
        Ok(detail) => Item {
            name: name.into(),
            ok: true,
            detail,
        },
        Err(e) => Item {
            name: name.into(),
            ok: false,
            detail: format!("{e:#}"),
        },
    }
}

pub fn preflight(agent: &dyn Agent, binary: &Path, can_build: bool) -> Vec<Item> {
    let mut items = Vec::new();
    items.push(item(
        "srt",
        tools::find_srt().map(|p| p.display().to_string()),
    ));
    items.push(item(
        "tmux",
        tools::which("tmux")
            .map(|p| p.display().to_string())
            .ok_or_else(|| anyhow::anyhow!("not on PATH (brew install tmux)")),
    ));
    items.push(item(&format!("{} CLI", agent.name()), agent.version()));
    // Presence only: the credential is never printed.
    items.push(item(
        &format!("{} credentials", agent.name()),
        agent.credential().map(|_| "found".to_string()),
    ));
    items.push(item(
        "cargo-agents",
        if binary.is_file() {
            Ok(binary.display().to_string())
        } else if can_build {
            Ok(format!(
                "{} missing; will be built by `run`",
                binary.display()
            ))
        } else {
            Err(anyhow::anyhow!("{} missing", binary.display()))
        },
    ));
    items.push(item(
        "rustup toolchain",
        tools::toolchain_bin().map(|p| p.display().to_string()),
    ));
    items.push(if tools::nested_sandbox() {
        Item {
            name: "not nested".into(),
            ok: false,
            detail: "already inside a macOS sandbox; srt cannot nest. Run this command from an unsandboxed terminal.".into(),
        }
    } else {
        Item { name: "not nested".into(), ok: true, detail: "no enclosing sandbox".into() }
    });
    items
}
