//! User-facing output with emoji status indicators.
//!
//! All user-visible status messages go through `Output`, which can be
//! silenced with `--quiet` or when running from a hook.

use std::path::Path;
use std::sync::{Arc, Mutex};

/// Controls whether user-facing status messages are printed.
#[derive(Debug, Clone)]
pub struct Output {
    quiet: bool,
    capture: Option<Arc<Mutex<Vec<String>>>>,
}

impl Output {
    // {impl}`design.requirements.output.messages_stdout`
    // {impl}`design.requirements.output.warnings_stderr`
    pub fn normal() -> Self {
        Self {
            quiet: false,
            capture: None,
        }
    }

    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.hooks.no_prompt`
    pub fn quiet() -> Self {
        Self {
            quiet: true,
            capture: None,
        }
    }

    /// Create an output that captures all messages into a buffer
    /// instead of printing them.
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.hooks.no_prompt`
    pub fn capturing() -> Self {
        Self {
            quiet: false,
            capture: Some(Arc::new(Mutex::new(Vec::new()))),
        }
    }

    /// Return all captured messages. Panics if not in capturing mode.
    // {impl}`design.requirements.output.capture`
    pub fn captured(&self) -> Vec<String> {
        self.capture
            .as_ref()
            .expect("not in capturing mode")
            .lock()
            .unwrap()
            .clone()
    }

    /// May we block on stdin to ask the user a question?
    ///
    /// Only a non-quiet, non-capturing output attached to a terminal on both
    /// ends qualifies. Hook dispatch and the library test harness both use
    /// quiet/capturing outputs, so neither can ever reach a prompt — the TTY
    /// check alone would not guarantee that, since `cargo test` inherits the
    /// developer's terminal.
    pub fn is_interactive(&self) -> bool {
        // {impl}`design.requirements.hooks.no_prompt`
        use std::io::IsTerminal;
        !self.quiet
            && self.capture.is_none()
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
    }

    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    fn emit(&self, msg: String) {
        if self.quiet {
            return;
        }
        if let Some(buf) = &self.capture {
            buf.lock().unwrap().push(msg);
        } else {
            println!("{msg}");
        }
    }

    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.warnings_stderr`
    fn emit_err(&self, msg: String) {
        if self.quiet {
            return;
        }
        if let Some(buf) = &self.capture {
            buf.lock().unwrap().push(msg);
        } else {
            eprintln!("{msg}");
        }
    }

    /// Something was already in place, no action needed.
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn already_ok(&self, msg: impl std::fmt::Display) {
        self.emit(format!("🟢 {msg}"));
    }

    /// An action was taken successfully (created, added, wrote).
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn done(&self, msg: impl std::fmt::Display) {
        self.emit(format!("✅ {msg}"));
    }

    /// A new item was discovered or added.
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn added(&self, msg: impl std::fmt::Display) {
        self.emit(format!("➕ {msg}"));
    }

    /// An item was removed.
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn removed(&self, msg: impl std::fmt::Display) {
        self.emit(format!("➖ {msg}"));
    }

    /// Informational status (not an action).
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn info(&self, msg: impl std::fmt::Display) {
        self.emit(format!("ℹ️  {msg}"));
    }

    /// A warning (something went wrong but not fatal).
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.warnings_stderr`
    pub fn warn(&self, msg: impl std::fmt::Display) {
        self.emit_err(format!("⚠️  {msg}"));
    }

    /// Print a blank line for spacing.
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn blank(&self) {
        self.emit(String::new());
    }

    /// Print arbitrary text (for prompts, headers, etc.).
    // {impl}`design.requirements.output.quiet`
    // {impl}`design.requirements.output.capture`
    // {impl}`design.requirements.output.messages_stdout`
    pub fn println(&self, msg: impl std::fmt::Display) {
        self.emit(format!("{msg}"));
    }
}

/// Replace the home directory prefix with `~` for display.
// {impl}`design.requirements.output.display_path`
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}
