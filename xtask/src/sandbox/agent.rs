//! The agent-specific half of the rig. One implementation (Claude); the trait
//! is shaped so another CLI (Codex) slots in without touching the runner.

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::matching;
use super::tools;

pub const KEYCHAIN_SERVICE_DEFAULT: &str = "symposium-srt-claude-token";
pub const KEYCHAIN_SERVICE_VAR: &str = "SANDBOX_CLAUDE_KEYCHAIN_SERVICE";
pub const TOKEN_VAR: &str = "SANDBOX_CLAUDE_TOKEN";

pub trait Agent {
    fn name(&self) -> &'static str;
    /// Directory holding the agent executable (the symlink's folder, since its
    /// target may be named after the version).
    fn bin_dir(&self) -> anyhow::Result<PathBuf>;
    /// Version string recorded in the summary.
    fn version(&self) -> anyhow::Result<String>;
    /// The credential, from the environment or the Keychain.
    fn credential(&self) -> anyhow::Result<String>;
    /// Write whatever the CLI needs in the fake HOME to skip onboarding.
    fn prepare_home(&self, home: &Path, project: &Path, version: &str) -> anyhow::Result<()>;
    /// Pre-create directories the sandbox could not create itself.
    fn prepare_project(&self, project: &Path) -> anyhow::Result<()>;
    /// The `--add-agent` value for `cargo agents init`.
    fn init_name(&self) -> &'static str;
    /// Command line to run under `srt`.
    fn launch_argv(&self, debug_log: &Path) -> Vec<String>;
    /// Name of the variable the launch script reads the credential from.
    fn credential_env(&self) -> &'static str;
    /// Extra environment for the launch script (besides the fixed allowlist).
    fn extra_env(&self, tmp: &Path) -> Vec<(String, String)>;
    /// Answer blocks in a pane capture, oldest first.
    fn answers(&self, pane: &str) -> Vec<String>;
    /// True once the input box is showing.
    fn input_ready(&self, pane: &str) -> bool;
    /// True while the agent is producing output.
    fn busy(&self, pane: &str) -> bool;
    /// Keys that empty the input box without exiting the agent.
    fn clear_input(&self) -> (&'static str, usize);
    fn exit_command(&self) -> &'static str;
}

pub fn by_name(name: &str) -> anyhow::Result<Box<dyn Agent>> {
    match name {
        "claude" => Ok(Box::new(Claude)),
        other => bail!("unsupported agent `{other}` (supported: claude)"),
    }
}

pub struct Claude;

impl Agent for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn bin_dir(&self) -> anyhow::Result<PathBuf> {
        tools::which("claude")
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .context("`claude` not found on PATH")
    }

    fn version(&self) -> anyhow::Result<String> {
        tools::output(Command::new(self.bin_dir()?.join("claude")).arg("--version"))
            .context("`claude --version` failed")
    }

    fn credential(&self) -> anyhow::Result<String> {
        if let Ok(t) = std::env::var(TOKEN_VAR)
            && !t.trim().is_empty()
        {
            return Ok(t.trim().to_string());
        }
        let service = std::env::var(KEYCHAIN_SERVICE_VAR)
            .unwrap_or_else(|_| KEYCHAIN_SERVICE_DEFAULT.to_string());
        let user = std::env::var("USER").unwrap_or_default();
        let from_keychain = tools::output(Command::new("security").args([
            "find-generic-password",
            "-a",
            &user,
            "-s",
            &service,
            "-w",
        ]));
        match from_keychain {
            Ok(t) if !t.is_empty() => Ok(t),
            _ => bail!(
                "no Claude credential: set {TOKEN_VAR}, or store a token in the Keychain \
                 (service `{service}`). Create one with `claude setup-token`."
            ),
        }
    }

    fn prepare_home(&self, home: &Path, project: &Path, version: &str) -> anyhow::Result<()> {
        let short = version.split_whitespace().next().unwrap_or(version);
        let json = serde_json::json!({
            "hasCompletedOnboarding": true,
            "lastOnboardingVersion": short,
            "theme": "dark",
            "projects": {
                project.display().to_string(): {
                    "hasTrustDialogAccepted": true,
                    "hasCompletedProjectOnboarding": true,
                }
            }
        });
        std::fs::write(
            home.join(".claude.json"),
            serde_json::to_string_pretty(&json)?,
        )?;
        Ok(())
    }

    fn prepare_project(&self, project: &Path) -> anyhow::Result<()> {
        // srt blocks creating any missing parent of its protected
        // `.claude/commands` and `.claude/agents`, so nothing inside the sandbox
        // can `mkdir .claude`. Variants control `.claude/skills`.
        std::fs::create_dir_all(project.join(".claude"))?;
        Ok(())
    }

    fn init_name(&self) -> &'static str {
        "claude"
    }

    fn launch_argv(&self, debug_log: &Path) -> Vec<String> {
        vec![
            "claude".into(),
            "--debug".into(),
            "--debug-file".into(),
            debug_log.display().to_string(),
        ]
    }

    fn credential_env(&self) -> &'static str {
        "CLAUDE_CODE_OAUTH_TOKEN"
    }

    fn extra_env(&self, tmp: &Path) -> Vec<(String, String)> {
        vec![
            ("DISABLE_AUTOUPDATER".into(), "1".into()),
            (
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                "1".into(),
            ),
            ("CLAUDE_CODE_TMPDIR".into(), tmp.display().to_string()),
        ]
    }

    fn answers(&self, pane: &str) -> Vec<String> {
        matching::claude_answer_blocks(pane)
    }

    fn input_ready(&self, pane: &str) -> bool {
        pane.lines().any(|l| l.trim_start().starts_with('❯'))
    }

    fn busy(&self, pane: &str) -> bool {
        pane.contains("esc to interrupt")
    }

    fn clear_input(&self) -> (&'static str, usize) {
        // Never Ctrl-C: it exits Claude Code here. Backspace on an empty box is a no-op.
        ("BSpace", 80)
    }

    fn exit_command(&self) -> &'static str {
        "/exit"
    }
}
