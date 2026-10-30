//! Audits: the audits file reader, the payload digest and its command, and
//! (below) how revocations and `disable` are enforced at activation.

use std::path::Path;

use symposium::audits::digest::payload_digest;
use symposium_testlib::{TestContext, TestMode, with_fixture};

/// Run `f` with a JSON report layer installed and return the drained events.
async fn with_report<F>(f: F) -> Vec<serde_json::Value>
where
    F: AsyncFnOnce(),
{
    use symposium::report::{ReportLayer, ReportMode};
    use tracing_subscriber::layer::SubscriberExt;

    symposium_testlib::install_tracing_baseline();
    let (layer, handle) = ReportLayer::new(ReportMode::Json, tracing::Level::INFO);
    let subscriber = tracing_subscriber::registry().with(layer);
    let guard = tracing::subscriber::set_default(subscriber);
    f().await;
    drop(guard);
    handle.drain()
}

/// `crate-d`'s source directory in the `audits0` fixture.
fn crate_d(ctx: &TestContext) -> std::path::PathBuf {
    ctx.workspace_root.clone().unwrap().join("deps/crate-d")
}

fn write_audits(registry: &Path, crate_file: &str, text: &str) {
    let dir = registry.join("audits");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(crate_file), text).unwrap();
}

/// A registry's `audits/` directory is never enumerated as plugins, even with
/// a `SKILL.md` and a `Symposium.toml` in it; the real entry still loads.
#[tokio::test]
async fn the_audits_directory_is_not_a_plugin_source() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |ctx| {
        let providers = symposium::plugins::list_plugins(&ctx.sym).await;
        let company = providers
            .iter()
            .find(|p| p.name == "company")
            .expect("the company registry is listed");
        let names: Vec<&str> = company.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["recommended"], "{providers:?}");

        let validated =
            symposium::plugins::validate_source_dir(&ctx.tempdir.join("company-audits"))?;
        assert_eq!(validated.len(), 1, "{validated:?}");
        Ok(())
    })
    .await
    .unwrap();
}

/// The lookup B2 uses: given a crate and a digest, which registry's audit
/// matched. Audits from several registries merge; the user's own
/// `~/.symposium/plugins/` counts like any other.
#[tokio::test]
async fn lookup_names_the_registry_whose_audit_matched() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |ctx| {
        let digest = payload_digest(&crate_d(&ctx)).unwrap();
        let other = "sha256:".to_string() + &"0".repeat(64);

        write_audits(
            &ctx.tempdir.join("company-audits"),
            "crate_d.toml",
            &format!("format = 1\ncrate = \"crate-d\"\n[[audit]]\ndigest = \"{digest}\"\n"),
        );
        write_audits(
            &ctx.sym.config_dir().join("plugins"),
            "crate_d.toml",
            &format!("format = 1\ncrate = \"crate-d\"\n[[audit]]\ndigest = \"{other}\"\n"),
        );

        let deps = ctx.sym.workspace_deps(ctx.workspace_root.as_ref().unwrap());
        let index = ctx.sym.package_managers(&deps).audit_index();
        assert_eq!(
            index.audited_by("crate-d", &digest).as_deref(),
            Some("company")
        );
        assert_eq!(
            index
                .audited_by("crate_d", &other.parse().unwrap())
                .as_deref(),
            Some("user-plugins")
        );
        let unknown = "sha256:".to_string() + &"1".repeat(64);
        assert_eq!(index.audited_by("crate-d", &unknown.parse().unwrap()), None);
        assert_eq!(index.audited_by("crate-x", &digest), None);
        Ok(())
    })
    .await
    .unwrap();
}

fn digest_event(events: &[serde_json::Value]) -> &serde_json::Value {
    events
        .iter()
        .find(|e| e["kind"] == "payload_digest")
        .unwrap_or_else(|| panic!("no payload_digest event in {events:?}"))
}

/// `plugin digest <dir>` and `plugin digest <crate>` agree with the library
/// function, and say which crate a name resolved to.
#[tokio::test]
async fn plugin_digest_prints_the_digest_for_a_directory_or_a_crate() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        let expected = payload_digest(&crate_d(&ctx)).unwrap().to_string();

        let events = with_report(async || {
            ctx.symposium(&["plugin", "digest", "crate-d"])
                .await
                .unwrap();
        })
        .await;
        let event = digest_event(&events);
        assert_eq!(event["digest"], expected.as_str());
        assert_eq!(event["krate"], "crate-d 1.2.0");

        // A directory, relative to the working directory.
        let events = with_report(async || {
            ctx.symposium(&["plugin", "digest", "crate-d"])
                .await
                .unwrap();
            ctx.symposium(&["plugin", "digest", "./deps/crate-d"])
                .await
                .unwrap();
        })
        .await;
        let dirs: Vec<_> = events
            .iter()
            .filter(|e| e["kind"] == "payload_digest")
            .collect();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[1]["digest"], expected.as_str());
        assert!(dirs[1].get("krate").is_none());
        Ok(())
    })
    .await
    .unwrap();
}

/// A payload that cannot be pinned reports why and fails the command.
#[tokio::test]
async fn plugin_digest_reports_why_a_payload_is_ineligible() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        std::fs::write(
            crate_d(&ctx).join("Symposium.toml"),
            "[[skills]]\nsource.git = \"https://github.com/example/skills\"\n",
        )?;
        let mut result = None;
        let events = with_report(async || {
            result = Some(ctx.symposium(&["plugin", "digest", "./deps/crate-d"]).await);
        })
        .await;
        assert!(result.unwrap().is_err(), "ineligible is a failure");
        let event = digest_event(&events);
        assert!(event.get("digest").is_none());
        assert!(
            event["ineligible"].as_str().unwrap().contains("source.git"),
            "{event}"
        );
        Ok(())
    })
    .await
    .unwrap();
}

// ── enforcement ──────────────────────────────────────────────────────

use std::path::PathBuf;

use symposium::config::{AutoAccept, Symposium, UseEntry};
use symposium::status_command::{StatusEntry, StatusState};

/// Every installed skill directory under `parent` named `<name>` or
/// `<name>-<hash>`.
fn installed(parent: &Path, name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == name || n.starts_with(&format!("{name}-")))
                && p.join("SKILL.md").is_file()
        })
        .collect()
}

/// The standing grants, each one enough on its own to load an unrevoked
/// crate-d.
type Grant = (&'static str, fn(&mut Symposium));

fn grants() -> Vec<Grant> {
    vec![
        ("auto-enable", |sym| {
            sym.config.plugins.auto_enable = vec!["crate-d".into()];
        }),
        ("use", |sym| {
            sym.config.plugins.used = vec![UseEntry::Global("crate-d".into())];
        }),
        ("auto-accept = all", |sym| {
            sym.config.plugins.auto_accept = AutoAccept::All;
        }),
    ]
}

async fn status(ctx: &TestContext) -> Vec<StatusEntry> {
    let deps = ctx.sym.workspace_deps(ctx.workspace_root.as_ref().unwrap());
    symposium::status_command::workspace_status(&ctx.sym, &deps)
        .await
        .unwrap()
}

fn entry<'a>(entries: &'a [StatusEntry], name: &str) -> &'a StatusEntry {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no status entry for `{name}` in {entries:?}"))
}

/// Run `audits` (the text of `audits/crate_d.toml` in the `company` registry,
/// given the payload digest) under each standing grant: the crate must not
/// load and must show as revoked; without the file the same grant loads it.
async fn assert_revoked_under_every_grant(audits: impl Fn(&str) -> String) {
    for (grant_name, grant) in grants() {
        with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
            grant(&mut ctx.sym);
            let digest = payload_digest(&crate_d(&ctx)).unwrap().to_string();
            let registry = ctx.tempdir.join("company-audits");

            write_audits(&registry, "crate_d.toml", &audits(&digest));
            ctx.symposium(&["sync"]).await?;
            assert!(
                installed(&skills, "d-guidance").is_empty(),
                "{grant_name}: a revoked crate must not load"
            );
            let entries = status(&ctx).await;
            let found = entry(&entries, "crate-d");
            assert_eq!(found.state, StatusState::Revoked, "{grant_name}: {found:?}");
            assert!(found.root.contains("`company`"), "{found:?}");
            assert!(
                found.root.contains("fetch and run remote code"),
                "the reason is rendered as a sentence: {found:?}"
            );

            // The grant itself is fine: lifting the revocation loads the crate.
            std::fs::remove_file(registry.join("audits/crate_d.toml"))?;
            ctx.symposium(&["sync"]).await?;
            assert_eq!(installed(&skills, "d-guidance").len(), 1, "{grant_name}");
            assert_eq!(
                entry(&status(&ctx).await, "crate-d").state,
                StatusState::Active,
                "{grant_name}"
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}

/// A revoked version range beats `use`, `auto-enable` and `auto-accept = all`.
#[tokio::test]
async fn a_crate_revoked_by_version_does_not_load() {
    assert_revoked_under_every_grant(|_| {
        "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \">=1.0.0, <1.3.0\"\n\
         reason = \"remote-exec\"\n"
            .to_string()
    })
    .await;
}

/// So does a revoked payload digest, which also covers a later release with
/// the same payload.
#[tokio::test]
async fn a_crate_revoked_by_digest_does_not_load() {
    assert_revoked_under_every_grant(|digest| {
        format!(
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\ndigest = \"{digest}\"\n\
             reason = \"remote-exec\"\n"
        )
    })
    .await;
}

/// Revocations that do not match, files that cannot be used, and entries that
/// are malformed all leave the crate loading.
#[tokio::test]
async fn revocations_that_do_not_apply_leave_the_crate_loaded() {
    let other = "sha256:".to_string() + &"f".repeat(64);
    let cases = [
        (
            "a version range that excludes it",
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"<1.0.0\"\nreason = \"report\"\n"
                .to_string(),
        ),
        (
            "another payload's digest",
            format!("format = 1\ncrate = \"crate-d\"\n[[revoked]]\ndigest = \"{other}\"\nreason = \"report\"\n"),
        ),
        (
            "an unknown format, which is ignored whole",
            "format = 2\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"*\"\nreason = \"report\"\n"
                .to_string(),
        ),
        (
            "an entry naming both versions and a digest, which is dropped",
            format!(
                "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"*\"\ndigest = \"{other}\"\nreason = \"report\"\n"
            ),
        ),
        (
            "an entry with a bad requirement, which is dropped",
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"nonsense\"\nreason = \"report\"\n"
                .to_string(),
        ),
        (
            "a file that is for another crate",
            "format = 1\ncrate = \"crate-x\"\n[[revoked]]\nversions = \"*\"\nreason = \"report\"\n"
                .to_string(),
        ),
    ];
    for (what, text) in cases {
        with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
            ctx.sym.config.plugins.auto_enable = vec!["crate-d".into()];
            write_audits(&ctx.tempdir.join("company-audits"), "crate_d.toml", &text);
            ctx.symposium(&["sync"]).await?;
            assert_eq!(installed(&skills, "d-guidance").len(), 1, "{what}");
            assert_eq!(
                entry(&status(&ctx).await, "crate-d").state,
                StatusState::Active,
                "{what}"
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}

/// A revocation from any registry applies, including `~/.symposium/plugins/`.
#[tokio::test]
async fn a_revocation_in_any_registry_applies() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
        ctx.sym.config.plugins.auto_enable = vec!["crate-d".into()];
        write_audits(
            &ctx.sym.config_dir().join("plugins"),
            "crate_d.toml",
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"*\"\nreason = \"injection-bait\"\n",
        );
        ctx.symposium(&["sync"]).await?;
        assert!(installed(&skills, "d-guidance").is_empty());
        let entries = status(&ctx).await;
        let found = entry(&entries, "crate-d");
        assert_eq!(found.state, StatusState::Revoked);
        assert!(found.root.contains("`user-plugins`"), "{found:?}");
        Ok(())
    })
    .await
    .unwrap();
}

/// `disable` is the last word: a revoked and disabled crate shows as declined,
/// and a disabled crate stays out whatever else consents to it.
#[tokio::test]
async fn disable_outranks_a_revocation_and_every_grant() {
    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
        ctx.sym.config.plugins.auto_enable = vec!["crate-d".into()];
        ctx.sym.config.plugins.disable = vec!["crate_d".into()];
        write_audits(
            &ctx.tempdir.join("company-audits"),
            "crate_d.toml",
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"*\"\nreason = \"report\"\n",
        );
        ctx.symposium(&["sync"]).await?;
        assert!(installed(&skills, "d-guidance").is_empty());
        assert_eq!(
            entry(&status(&ctx).await, "crate-d").state,
            StatusState::Declined
        );
        Ok(())
    })
    .await
    .unwrap();
}

/// `chained-crate0`: a registry plugin vouches for `crate-w` through a
/// `[[plugins]]` chained reference, so `crate-w` loads with no `[plugins]`
/// entry of its own.
async fn chained_skill_installs(ctx: &mut TestContext) -> usize {
    ctx.symposium(&["sync"]).await.unwrap();
    let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
    installed(&skills, "w-guidance").len()
}

/// `disable` stops registry plugins and chained crates, which it did not
/// before: the chain's target is skipped, and so is a registry plugin.
#[tokio::test]
async fn disable_stops_chained_crates_and_registry_plugins() {
    for disabled in ["crate-w", "crate_w", "vouch-plugin"] {
        with_fixture(
            TestMode::SimulationOnly,
            &["chained-crate0"],
            async |mut ctx| {
                ctx.symposium(&["init", "--add-agent", "claude"]).await?;
                assert_eq!(chained_skill_installs(&mut ctx).await, 1, "baseline");
                ctx.sym.config.plugins.disable = vec![disabled.into()];
                assert_eq!(
                    chained_skill_installs(&mut ctx).await,
                    0,
                    "disable {disabled}"
                );
                Ok(())
            },
        )
        .await
        .unwrap();
    }
}

/// A revoked crate is not loaded even through a chained reference (the
/// registry vouches for it, but a revocation is the registry's own word).
#[tokio::test]
async fn a_revoked_chained_crate_is_not_loaded() {
    with_fixture(TestMode::SimulationOnly, &["chained-crate0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        assert_eq!(chained_skill_installs(&mut ctx).await, 1, "baseline");
        write_audits(
            &ctx.sym.config_dir().join("plugins"),
            "crate_w.toml",
            "format = 1\ncrate = \"crate-w\"\n[[revoked]]\nversions = \"<1.0.0\"\nreason = \"exfiltration\"\n",
        );
        assert_eq!(chained_skill_installs(&mut ctx).await, 0);
        Ok(())
    })
    .await
    .unwrap();
}

/// A revocation covers cargo-sourced plugins only: an audits file naming a
/// registry plugin (or a workspace one) does not switch it off.
#[tokio::test]
async fn revocations_do_not_apply_to_registry_plugins() {
    with_fixture(TestMode::SimulationOnly, &["chained-crate0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        write_audits(
            &ctx.sym.config_dir().join("plugins"),
            "vouch_plugin.toml",
            "format = 1\ncrate = \"vouch-plugin\"\n[[revoked]]\nversions = \"*\"\nreason = \"report\"\n",
        );
        assert_eq!(chained_skill_installs(&mut ctx).await, 1);
        Ok(())
    })
    .await
    .unwrap();
}

/// The hook path keeps agent stdout clean when a plugin is revoked, and does
/// not load it.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_crate_leaves_hook_stdout_alone() {
    use std::io::Write;
    use symposium::hook_schema::{HookAgent, agent_event};
    use symposium_testlib::HookStep;

    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        let workspace = ctx.workspace_root.clone().unwrap();
        let skills = workspace.join(".claude/skills");
        let config = ctx.sym.config_dir().join("config.toml");
        let text = std::fs::read_to_string(&config)?;
        std::fs::write(
            &config,
            format!("{text}\n[plugins]\nauto-enable = [\"crate-d\"]\n"),
        )?;
        let registry = ctx.tempdir.join("company-audits");
        write_audits(
            &registry,
            "crate_d.toml",
            "format = 1\ncrate = \"crate-d\"\n[[revoked]]\nversions = \"*\"\nreason = \"report\"\n\
             [[revoked]]\nversions = \"broken\"\n",
        );

        let step = HookStep::session_start();
        let payload = agent_event(HookAgent::Claude, step.event())
            .unwrap()
            .translate_input(&step.to_input_event(&workspace.display().to_string()))
            .to_string()
            .unwrap();
        let run_hook = || {
            let binary =
                std::env::var("CARGO_BIN_EXE_cargo-agents").expect("must run via cargo test");
            let mut child = std::process::Command::new(binary)
                .args(["hook", "claude", "session-start"])
                .env("SYMPOSIUM_HOME", ctx.sym.config_dir())
                .current_dir(&workspace)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn cargo-agents");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if !stdout.trim().is_empty() {
                serde_json::from_str::<serde_json::Value>(&stdout)
                    .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {stdout}"));
            }
            assert!(!stdout.contains("revoked"), "{stdout}");
        };

        run_hook();
        assert!(installed(&skills, "d-guidance").is_empty());

        // The hook does sync, so without the revocation the same setup
        // installs the skill: the assertion above is not vacuous.
        std::fs::remove_file(registry.join("audits/crate_d.toml"))?;
        run_hook();
        assert_eq!(installed(&skills, "d-guidance").len(), 1);
        Ok(())
    })
    .await
    .unwrap();
}
