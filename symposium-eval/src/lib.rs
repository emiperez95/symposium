//! Agent evaluations: run a real agent client on a small Rust task, with or
//! without Symposium, then grade the code it leaves behind and check what
//! Symposium did along the way.
//!
//! This crate is the engine behind `cargo xtask agent-eval`. The approved design
//! is `md/rfds/skill-evaluation/`; `md/design/agent-eval.md` maps it onto this
//! crate's modules.

use std::ffi::OsString;

use anyhow::bail;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "agent-eval",
    bin_name = "cargo xtask agent-eval",
    about = "Evaluate agent tasks with and without Symposium"
)]
struct Cli {}

/// `args` is everything after `agent-eval`. Clap handles `--help` and invalid
/// arguments by printing and exiting the process.
///
/// # Errors
///
/// Only when the runner itself fails. Whatever happens inside a trial is an
/// outcome, not an error.
pub fn run<I, T>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let program = OsString::from("agent-eval");
    Cli::parse_from(std::iter::once(program).chain(args.into_iter().map(Into::into)));
    bail!("agent-eval has no commands yet; see md/rfds/skill-evaluation/README.md for the plan")
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
