//! `cargo xtask sandbox`: run a real agent CLI against Symposium inside an
//! `srt` sandbox.
//!
//! Division of labour: `srt` is the fence (OS sandbox, file rules, network
//! allowlist). This module is the rig around it: a clean fake machine, the
//! agent-specific setup, driving the interactive CLI through tmux, credentials,
//! checking answers and debug logs, and run folders with verdicts.
//! See `xtask/sandbox/README.md`.

mod agent;
mod check;
mod matching;
mod overlay;
mod policy;
mod run;
mod scenario;
mod summary;
mod tmux;
mod tools;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "cargo xtask sandbox",
    about = "Run real agent CLIs against Symposium in an srt sandbox"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Preflight: srt, tmux, the agent CLI, credentials, the binary, nesting.
    Check {
        #[arg(long, default_value = "claude")]
        agent: String,
        /// `cargo-agents` binary to test (default: target/debug/cargo-agents).
        #[arg(long, env = "SYMPOSIUM_SANDBOX_BINARY")]
        binary: Option<PathBuf>,
    },
    /// List scenarios and their variants.
    List,
    /// Run a scenario.
    Run {
        scenario: String,
        /// Variant to run; repeatable. Default: all.
        #[arg(long = "variant")]
        variants: Vec<String>,
        /// Tag for the run folder.
        #[arg(long, default_value = "run")]
        label: String,
        #[arg(long, default_value = "claude")]
        agent: String,
        /// Keep the short TMPDIR too (the run folder is always kept).
        #[arg(long)]
        keep: bool,
        /// Do not build `cargo-agents` first.
        #[arg(long)]
        no_build: bool,
        /// `cargo-agents` binary to test; implies --no-build.
        #[arg(long, env = "SYMPOSIUM_SANDBOX_BINARY")]
        binary: Option<PathBuf>,
    },
}

pub fn main(project_root: &Path, args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse_from(
        std::iter::once("cargo xtask sandbox".to_string()).chain(args.iter().cloned()),
    );
    let scenarios_dir = project_root.join("xtask/sandbox/scenarios");
    match cli.command {
        Cmd::List => list(&scenarios_dir),
        Cmd::Check { agent, binary } => {
            let agent = agent::by_name(&agent)?;
            let binary = binary.unwrap_or_else(|| default_binary(project_root));
            let items = check::preflight(agent.as_ref(), &binary, true);
            let mut bad = 0;
            for i in &items {
                println!(
                    "{} {:<20} {}",
                    if i.ok { "ok  " } else { "FAIL" },
                    i.name,
                    i.detail
                );
                if !i.ok {
                    bad += 1;
                }
            }
            if bad > 0 {
                bail!("{bad} preflight check(s) failed");
            }
            Ok(())
        }
        Cmd::Run {
            scenario,
            variants,
            label,
            agent,
            keep,
            no_build,
            binary,
        } => run_scenario(
            project_root,
            &scenarios_dir,
            &scenario,
            &variants,
            &label,
            &agent,
            keep,
            no_build,
            binary,
        ),
    }
}

fn default_binary(root: &Path) -> PathBuf {
    root.join("target/debug/cargo-agents")
}

fn list(dir: &Path) -> anyhow::Result<()> {
    for name in scenario::list(dir)? {
        let s = scenario::Scenario::load(dir, &name)?;
        let variants: Vec<&str> = s.variants.iter().map(|v| v.name.as_str()).collect();
        println!("{name}: {}", s.description);
        println!("  variants: {}", variants.join(", "));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_scenario(
    root: &Path,
    dir: &Path,
    name: &str,
    wanted: &[String],
    label: &str,
    agent_name: &str,
    keep: bool,
    no_build: bool,
    binary: Option<PathBuf>,
) -> anyhow::Result<()> {
    if label.is_empty()
        || !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("label must be [A-Za-z0-9_-]+");
    }
    let scenario = scenario::Scenario::load(dir, name)?;
    let agent = agent::by_name(&scenario_agent(&scenario, agent_name))?;

    let mut indexes = Vec::new();
    if wanted.is_empty() {
        indexes.extend(0..scenario.variants.len());
    } else {
        for w in wanted {
            let i = scenario
                .variants
                .iter()
                .position(|v| &v.name == w)
                .with_context(|| format!("scenario `{name}` has no variant `{w}`"))?;
            indexes.push(i);
        }
    }

    if tools::nested_sandbox() {
        bail!(
            "already inside a macOS sandbox; srt cannot nest. Run this command from an unsandboxed terminal."
        );
    }
    let tools = tools::Tools::discover()?;

    let (binary, build) = match binary {
        Some(b) => (b, false),
        None => (default_binary(root), !no_build),
    };
    if build {
        println!("building cargo-agents ...");
        let status = Command::new("cargo")
            .args(["build", "--bin", "cargo-agents"])
            .current_dir(root)
            .status()?;
        if !status.success() {
            bail!("cargo build failed");
        }
    }
    if binary.file_name().is_none_or(|n| n != "cargo-agents") {
        bail!(
            "{} must be named `cargo-agents` (it is put on PATH by directory)",
            binary.display()
        );
    }
    let binary = std::fs::canonicalize(&binary).with_context(|| {
        format!(
            "{} not found; build it or drop --no-build",
            binary.display()
        )
    })?;

    let fixtures_base = root.join("tests/fixtures");
    let mut failed = 0;
    for i in indexes {
        let cfg = run::RunConfig {
            scenario_name: name,
            scenario: &scenario,
            variant: i,
            label,
            binary: &binary,
            fixtures_base: &fixtures_base,
            agent: agent.as_ref(),
            tools: &tools,
            keep,
        };
        match run::run_variant(&cfg) {
            Ok(out) => {
                println!("{}", summary::verdict_line(&out.summary, &out.run_dir));
                if !out.summary.passed() {
                    failed += 1;
                }
            }
            Err(e) => {
                println!("ERROR {name}/{}: {e:#}", scenario.variants[i].name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} variant(s) failed");
    }
    Ok(())
}

/// `--agent` is the default; a scenario may pin another in `[symposium] agent`.
/// The flag wins when it is not the default value.
fn scenario_agent(s: &scenario::Scenario, flag: &str) -> String {
    if flag != "claude" {
        flag.to_string()
    } else {
        s.symposium.agent.clone()
    }
}
