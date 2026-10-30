//! Running one variant of a scenario end to end.

use anyhow::{Context, bail};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::agent::Agent;
use super::matching;
use super::overlay::{self, Vars};
use super::policy;
use super::scenario::{Scenario, Step};
use super::summary::{Check, Summary, Versions, verdict_of};
use super::tmux::Tmux;
use super::tools::{self, Tools, sh_quote};

pub const WORK_ROOT: &str = "/tmp/symposium-sandbox";

pub struct RunConfig<'a> {
    pub scenario_name: &'a str,
    pub scenario: &'a Scenario,
    pub variant: usize,
    pub label: &'a str,
    pub binary: &'a Path,
    pub fixtures_base: &'a Path,
    pub agent: &'a dyn Agent,
    pub tools: &'a Tools,
    pub keep: bool,
}

pub struct Outcome {
    pub summary: Summary,
    pub run_dir: PathBuf,
}

fn work_dir(sub: &str) -> anyhow::Result<PathBuf> {
    let p = Path::new(WORK_ROOT).join(sub);
    fs::create_dir_all(&p)?;
    // /tmp is /private/tmp on macOS: srt and the agent see the canonical path.
    Ok(fs::canonicalize(&p)?)
}

/// Remove a folder this tool created. Refuses anything outside [`WORK_ROOT`].
fn remove_own(path: &Path) -> anyhow::Result<()> {
    let root = fs::canonicalize(WORK_ROOT)?;
    let canon = match fs::canonicalize(path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // nothing there
    };
    if !canon.starts_with(&root) || canon == root {
        bail!(
            "refusing to delete {} (not under {})",
            canon.display(),
            root.display()
        );
    }
    fs::remove_dir_all(&canon)?;
    Ok(())
}

pub fn run_variant(cfg: &RunConfig) -> anyhow::Result<Outcome> {
    let variant = &cfg.scenario.variants[cfg.variant];
    let run_name = format!("{}-{}-{}", cfg.scenario_name, variant.name, cfg.label);

    let runs = work_dir("runs")?;
    let run = runs.join(&run_name);
    remove_own(&run)?;
    let tmp_parent = work_dir("tmp")?;
    let tmp = tmp_parent.join(&run_name);
    remove_own(&tmp)?;

    let home = run.join("home");
    let project = run.join("project");
    let cargo_home = run.join("cargo-home");
    for d in [&home, &project, &cargo_home, &tmp] {
        fs::create_dir_all(d)?;
    }
    let (run, home, project, cargo_home, tmp) = (
        fs::canonicalize(&run)?,
        fs::canonicalize(&home)?,
        fs::canonicalize(&project)?,
        fs::canonicalize(&cargo_home)?,
        fs::canonicalize(&tmp)?,
    );

    let result = drive(cfg, &run, &home, &project, &cargo_home, &tmp);

    // Only the short TMPDIR is disposable; the run folder is kept (always, not
    // only on failure: it is the evidence, and `--keep` is for the TMPDIR too).
    if !cfg.keep {
        let _ = remove_own(&tmp);
    }
    result.map(|summary| Outcome {
        summary,
        run_dir: run,
    })
}

fn drive(
    cfg: &RunConfig,
    run: &Path,
    home: &Path,
    project: &Path,
    cargo_home: &Path,
    tmp: &Path,
) -> anyhow::Result<Summary> {
    let s = cfg.scenario;
    let variant = &s.variants[cfg.variant];
    let agent = cfg.agent;
    let tools = cfg.tools;
    let tmux = Tmux {
        bin: tools.tmux.clone(),
    };

    // Everything needed before anything starts.
    let credential = agent.credential()?;
    let agent_version = agent.version()?;
    let bin_dir = cfg.binary.parent().context("binary has no parent")?;
    let path_var = std::env::join_paths([
        bin_dir.to_path_buf(),
        tools.toolchain_bin.clone(),
        agent.bin_dir()?,
        tools.node_bin.clone(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ])?
    .to_string_lossy()
    .into_owned();

    // Project and fake HOME.
    let vars = Vars {
        test_dir: project,
        binary: cfg.binary,
        home,
    };
    overlay::overlay_fixtures(cfg.fixtures_base, &s.fixtures, project, home, &vars)?;
    overlay::write_inline(&s.files, project, home, &vars)?;
    overlay::write_inline(&variant.files, project, home, &vars)?;
    for d in &variant.dirs {
        fs::create_dir_all(overlay::resolve(d, project, home)?)?;
    }
    agent.prepare_project(project)?;
    agent.prepare_home(home, project, &agent_version)?;

    // Symposium config goes in first so `init` (flags given, so no prompts) keeps it.
    if let Some(config) = &s.symposium.config {
        let dir = home.join(".symposium");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("config.toml"), overlay::expand(config, &vars))?;
    }
    let init = Command::new(cfg.binary)
        .args([
            "init",
            "--add-agent",
            agent.init_name(),
            "--hook-scope",
            &s.symposium.hook_scope,
            "-q",
        ])
        .current_dir(project)
        .env_clear()
        .env("HOME", home)
        .env("PATH", &path_var)
        .env("TMPDIR", tmp)
        .env("CARGO_HOME", cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .context("running `cargo-agents init`")?;
    if !init.status.success() {
        bail!(
            "`cargo-agents init` failed: {}",
            String::from_utf8_lossy(&init.stderr).trim()
        );
    }

    // Sandbox policy.
    let srt_json = run.join("srt.json");
    fs::write(
        &srt_json,
        serde_json::to_string_pretty(&policy::srt_settings(
            &tools.real_home,
            run,
            tmp,
            &s.network,
        ))?,
    )?;

    // Launch script. It holds no secret: the credential arrives in $SANDBOX_SECRET.
    let debug_log = run.join("agent-debug.log");
    let mut envs: Vec<(String, String)> = vec![
        ("HOME".into(), home.display().to_string()),
        ("USER".into(), std::env::var("USER").unwrap_or_default()),
        ("TERM".into(), "xterm-256color".into()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("PATH".into(), path_var.clone()),
        ("TMPDIR".into(), tmp.display().to_string()),
        ("CARGO_HOME".into(), cargo_home.display().to_string()),
        ("CARGO_NET_OFFLINE".into(), "true".into()),
    ];
    envs.extend(agent.extra_env(tmp));
    let mut script = String::from(
        "#!/bin/sh\n# Generated by `cargo xtask sandbox`. Holds no secret: the credential arrives as $SANDBOX_SECRET.\n",
    );
    script.push_str(&format!(
        "cd {}\nenv -i",
        sh_quote(&project.display().to_string())
    ));
    for (k, v) in &envs {
        script.push_str(&format!(" \\\n  {k}={}", sh_quote(v)));
    }
    script.push_str(&format!(
        " \\\n  {}=\"$SANDBOX_SECRET\"",
        agent.credential_env()
    ));
    script.push_str(&format!(
        " \\\n  {} --settings {} --",
        sh_quote(&tools.srt.display().to_string()),
        sh_quote(&srt_json.display().to_string())
    ));
    for a in agent.launch_argv(&debug_log) {
        script.push_str(&format!(" {}", sh_quote(&a)));
    }
    script.push_str("\necho \"[agent exited $?]\"; sleep 600\n");
    let launch = run.join("launch.sh");
    fs::write(&launch, script)?;

    let srt_version = tools::output(Command::new(&tools.srt).arg("--version"))
        .unwrap_or_else(|_| "unknown".into());
    let versions = Versions {
        agent: agent_version.clone(),
        srt: srt_version,
        symposium_commit: tools::symposium_commit(bin_dir),
    };

    let session = format!("{}-{}-{}", cfg.scenario_name, variant.name, cfg.label);
    tmux.start(&session, project, &launch, ("SANDBOX_SECRET", &credential))?;
    drop(credential);

    let mut checks: Vec<Check> = Vec::new();
    let driven = drive_steps(
        cfg,
        &tmux,
        &session,
        run,
        home,
        project,
        &debug_log,
        &mut checks,
    );
    // Always tear the session down, whatever happened.
    let _ = tmux.send_literal(&session, agent.exit_command());
    std::thread::sleep(Duration::from_secs(1));
    let _ = tmux.send_enter(&session);
    std::thread::sleep(Duration::from_secs(3));
    let _ = tmux.kill(&session);
    if let Err(e) = driven {
        checks.push(Check {
            name: "run completed".into(),
            passed: false,
            detail: format!("{e:#}"),
        });
    }

    let summary = Summary {
        scenario: cfg.scenario_name.to_string(),
        variant: variant.name.clone(),
        label: cfg.label.to_string(),
        agent: agent.name().to_string(),
        started_at: now_utc(),
        versions,
        verdict: verdict_of(&checks).to_string(),
        checks,
    };
    fs::write(
        run.join("summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;
    Ok(summary)
}

fn now_utc() -> String {
    tools::output(Command::new("date").args(["-u", "+%FT%TZ"])).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn drive_steps(
    cfg: &RunConfig,
    tmux: &Tmux,
    session: &str,
    run: &Path,
    home: &Path,
    project: &Path,
    debug_log: &Path,
    checks: &mut Vec<Check>,
) -> anyhow::Result<()> {
    let agent = cfg.agent;
    let t = cfg.scenario.timeouts;
    let save = |name: &str, pane: &str| {
        let _ = fs::write(run.join(name), pane);
    };

    // Startup: wait for the input box.
    let start = Instant::now();
    let mut pane = String::new();
    while start.elapsed() < Duration::from_secs(t.startup) {
        pane = tmux.capture(session);
        if agent.input_ready(&pane) {
            break;
        }
        if pane.contains("[agent exited") {
            save("pane-startup.txt", &pane);
            bail!("agent exited during startup (see pane-startup.txt)");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    save("pane-startup.txt", &pane);
    if !agent.input_ready(&pane) {
        bail!(
            "input box not shown within {}s (see pane-startup.txt)",
            t.startup
        );
    }

    let mut last_answer: Option<String> = None;
    for (i, step) in cfg.scenario.steps.iter().enumerate() {
        let n = i + 1;
        match step {
            Step::Wait(secs) => std::thread::sleep(Duration::from_secs(*secs)),
            Step::Prompt(text) => {
                let (key, count) = agent.clear_input();
                tmux.send_keys(session, count, key)?;
                std::thread::sleep(Duration::from_secs(1));
                let before = agent.answers(&tmux.capture(session)).len();
                tmux.paste(session, text)?;
                std::thread::sleep(Duration::from_secs(2));
                save(
                    &format!("pane-{n:02}-before-enter.txt"),
                    &tmux.capture(session),
                );
                tmux.send_enter(session)?;

                // Done when a new answer is present, the agent is idle again and the
                // pane has stopped changing.
                let begin = Instant::now();
                let mut prev = String::new();
                let mut stable = 0;
                let mut pane;
                let mut finished = false;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    pane = tmux.capture(session);
                    let has_answer = agent.answers(&pane).len() > before;
                    if has_answer && !agent.busy(&pane) && pane == prev {
                        stable += 1;
                        if stable >= 2 {
                            finished = true;
                            break;
                        }
                    } else {
                        stable = 0;
                    }
                    prev = pane.clone();
                    if begin.elapsed() > Duration::from_secs(t.answer) {
                        break;
                    }
                }
                save(&format!("pane-{n:02}-after-prompt.txt"), &pane);
                let blocks = agent.answers(&pane);
                last_answer = Some(blocks.get(before..).unwrap_or(&[]).join("\n"));
                checks.push(Check {
                    name: format!("step {n}: answer arrived"),
                    passed: finished,
                    detail: if finished {
                        format!("after {}s", begin.elapsed().as_secs())
                    } else {
                        format!("no finished answer within {}s", t.answer)
                    },
                });
            }
            Step::ExpectAnswer(pattern) => {
                let re = Regex::new(pattern)?;
                let (passed, detail) = match &last_answer {
                    None => (false, "no prompt was sent before this step".to_string()),
                    Some(a) => (re.is_match(a), format!("answer: {}", first_line(a))),
                };
                checks.push(Check {
                    name: format!("step {n}: answer matches /{pattern}/"),
                    passed,
                    detail,
                });
            }
            Step::ExpectLog { pattern, count } | Step::RejectLog { pattern, count } => {
                let reject = matches!(step, Step::RejectLog { .. });
                let re = Regex::new(pattern)?;
                let log = fs::read_to_string(debug_log).unwrap_or_default();
                let (passed, detail) = matching::check_log(&re, &log, *count, reject);
                let verb = if reject { "log rejects" } else { "log has" };
                checks.push(Check {
                    name: format!("step {n}: {verb} /{pattern}/"),
                    passed,
                    detail,
                });
            }
            Step::ExpectFile(rel) => {
                let path = overlay::resolve(rel, project, home)?;
                let passed = path.is_file();
                checks.push(Check {
                    name: format!("step {n}: file {rel}"),
                    passed,
                    detail: if passed {
                        "exists".into()
                    } else {
                        format!("missing: {}", path.display())
                    },
                });
            }
        }
    }
    save("pane-final.txt", &tmux.capture(session));
    Ok(())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(80).collect()
}
