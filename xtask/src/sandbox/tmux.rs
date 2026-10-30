//! A private tmux server. The user's default server runs real sessions and
//! leaks its environment, so it is never touched.

use anyhow::bail;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const SOCKET: &str = "symposium-sandbox";

pub struct Tmux {
    pub bin: PathBuf,
}

impl Tmux {
    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(["-L", SOCKET, "-f", "/dev/null"]);
        c
    }

    fn run(&self, args: &[&str]) -> anyhow::Result<String> {
        let out = self.cmd().args(args).output()?;
        if !out.status.success() {
            bail!(
                "tmux {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Start a detached session running `/bin/sh <script>`. `secret_env` goes in
    /// through `-e` only: never argv of the script, never disk.
    pub fn start(
        &self,
        session: &str,
        cwd: &std::path::Path,
        script: &std::path::Path,
        secret_env: (&str, &str),
    ) -> anyhow::Result<()> {
        let _ = self.kill(session);
        let env = format!("{}={}", secret_env.0, secret_env.1);
        let out = self
            .cmd()
            .args([
                "new-session",
                "-d",
                "-s",
                session,
                "-x",
                "200",
                "-y",
                "50",
                "-c",
            ])
            .arg(cwd)
            .args(["-e", &env])
            .arg(format!(
                "/bin/sh {}",
                super::tools::sh_quote(&script.display().to_string())
            ))
            .stderr(Stdio::piped())
            .output()?;
        if !out.status.success() {
            // tmux echoes nothing secret, but be careful regardless.
            bail!(
                "tmux new-session failed: {}",
                String::from_utf8_lossy(&out.stderr).replace(secret_env.1, "***")
            );
        }
        Ok(())
    }

    pub fn capture(&self, session: &str) -> String {
        self.run(&["capture-pane", "-p", "-S", "-2000", "-t", session])
            .unwrap_or_default()
    }

    pub fn send_keys(&self, session: &str, count: usize, key: &str) -> anyhow::Result<()> {
        self.run(&["send-keys", "-t", session, "-N", &count.to_string(), key])
            .map(|_| ())
    }

    pub fn send_enter(&self, session: &str) -> anyhow::Result<()> {
        self.run(&["send-keys", "-t", session, "Enter"]).map(|_| ())
    }

    pub fn send_literal(&self, session: &str, text: &str) -> anyhow::Result<()> {
        self.run(&["send-keys", "-t", session, "-l", text])
            .map(|_| ())
    }

    /// Paste as one bracketed paste rather than per-key typing.
    pub fn paste(&self, session: &str, text: &str) -> anyhow::Result<()> {
        self.run(&["set-buffer", "-b", "sandbox", text])?;
        self.run(&["paste-buffer", "-p", "-d", "-b", "sandbox", "-t", session])
            .map(|_| ())
    }

    pub fn kill(&self, session: &str) -> anyhow::Result<()> {
        self.run(&["kill-session", "-t", session]).map(|_| ())
    }
}
