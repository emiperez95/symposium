//! `[[plugins]] source.cargo` edges and consent. An edge declared by a registry
//! or workspace plugin is vouched for; one declared by a dependency-sourced
//! plugin names a crate that loads only if the user enabled it or a trusted
//! registry audited it. Fixture `chain-consent0`: `crate-p` chains to
//! `crate-t`, which chains to `crate-u`; all three are workspace dependencies.

use std::path::{Path, PathBuf};

use symposium::audits::digest::payload_digest;
use symposium::config::{AutoAccept, Symposium, UseEntry};
use symposium_testlib::{TestContext, TestMode, with_fixture};

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

/// Sync, then report which of `p`, `t`, `u` have their skill installed.
async fn synced(ctx: &mut TestContext) -> String {
    ctx.symposium(&["sync"]).await.unwrap();
    let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
    ["p", "t", "u"]
        .into_iter()
        .filter(|n| !installed(&skills, &format!("{n}-guidance")).is_empty())
        .collect()
}

fn write_audits(ctx: &TestContext, file: &str, text: &str) {
    let dir = ctx.sym.config_dir().join("plugins/audits");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), text).unwrap();
}

fn crate_dir(ctx: &TestContext, name: &str) -> PathBuf {
    ctx.workspace_root.clone().unwrap().join(name)
}

/// What lets `crate-t` load when `crate-p`'s plugin names it.
type Grant = (&'static str, fn(&mut Symposium));

fn grants_for_t() -> Vec<Grant> {
    vec![
        ("use", |sym| {
            sym.config.plugins.used = vec![UseEntry::Global("crate-t".into())];
        }),
        ("auto-enable", |sym| {
            sym.config.plugins.auto_enable = vec!["crate-p".into(), "crate-t".into()];
        }),
        ("auto-accept = all", |sym| {
            sym.config.plugins.auto_accept = AutoAccept::All;
        }),
    ]
}

/// `crate-p` enabled, nothing else: its edge is unvouched, so `crate-t` does not
/// load (and so neither does `crate-u`, behind it).
#[tokio::test]
async fn an_unaccepted_chain_target_is_not_loaded() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            assert_eq!(synced(&mut ctx).await, "p");
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// Each way of accepting a crate accepts a chain target too. The target's own
/// edge is unvouched in turn: `crate-u` stays out.
#[tokio::test]
async fn every_grant_loads_the_chain_target_but_not_the_chain_behind_it() {
    for (name, grant) in grants_for_t() {
        with_fixture(
            TestMode::SimulationOnly,
            &["chain-consent0"],
            async |mut ctx| {
                ctx.symposium(&["init", "--add-agent", "claude"]).await?;
                ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
                grant(&mut ctx.sym);
                let loaded = synced(&mut ctx).await;
                if name == "auto-accept = all" {
                    // `all` accepts every unvouched crate, so the whole chain loads.
                    assert_eq!(loaded, "ptu", "{name}");
                } else {
                    assert_eq!(loaded, "pt", "{name}");
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    }
}

/// `use` names the end of a chain of chains: it loads, the middle stays out.
#[tokio::test]
async fn a_use_entry_for_the_far_end_does_not_vouch_for_the_middle() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            ctx.sym.config.plugins.used = vec![UseEntry::Global("crate-u".into())];
            assert_eq!(synced(&mut ctx).await, "pu");
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// A matching audit in a trusted registry accepts the chain target.
#[tokio::test]
async fn an_audited_chain_target_loads() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            ctx.sym.config.plugins.auto_accept = AutoAccept::Audit;
            let digest = payload_digest(&crate_dir(&ctx, "crate-t")).unwrap();
            write_audits(
                &ctx,
                "crate_t.toml",
                &format!("format = 1\ncrate = \"crate-t\"\n[[audit]]\ndigest = \"{digest}\"\n"),
            );
            // `crate-u` carries no audit, and neither is `crate-t`'s audit for it.
            assert_eq!(synced(&mut ctx).await, "pt");

            // An audit of some other payload accepts nothing.
            write_audits(
                &ctx,
                "crate_t.toml",
                &format!(
                    "format = 1\ncrate = \"crate-t\"\n[[audit]]\ndigest = \"sha256:{}\"\n",
                    "0".repeat(64)
                ),
            );
            assert_eq!(synced(&mut ctx).await, "p");
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// `disable` and a revocation beat every grant for a chain target.
#[tokio::test]
async fn disable_and_revocation_beat_every_grant_for_a_chain_target() {
    for (name, grant) in grants_for_t() {
        with_fixture(TestMode::SimulationOnly, &["chain-consent0"], async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            grant(&mut ctx.sym);
            assert!(synced(&mut ctx).await.contains('t'), "{name}: baseline");

            ctx.sym.config.plugins.disable = vec!["crate-t".into()];
            assert!(!synced(&mut ctx).await.contains('t'), "{name}: disable");
            ctx.sym.config.plugins.disable = vec![];

            write_audits(
                &ctx,
                "crate_t.toml",
                "format = 1\ncrate = \"crate-t\"\n[[revoked]]\nversions = \"*\"\nreason = \"exfiltration\"\n",
            );
            assert!(!synced(&mut ctx).await.contains('t'), "{name}: revoked");
            std::fs::remove_dir_all(ctx.sym.config_dir().join("plugins/audits")).unwrap();
            Ok(())
        })
        .await
        .unwrap();
    }
}

/// A registry plugin's edge is vouched for: `crate-t` loads with no consent,
/// while `crate-u` (named by `crate-t`) still needs its own.
#[tokio::test]
async fn a_registry_plugin_vouches_for_its_chain_target() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            add_registry_plugin(&ctx);
            assert_eq!(synced(&mut ctx).await, "t");
            Ok(())
        },
    )
    .await
    .unwrap();
}

fn add_registry_plugin(ctx: &TestContext) {
    let dir = ctx.sym.config_dir().join("plugins/vouch");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Symposium.toml"),
        "name = \"vouch\"\ndepends-on = [\"crate-p\"]\n\n[[plugins]]\nsource.cargo = \"crate-t\"\n",
    )
    .unwrap();
}

/// `crate-t` is reached by a registry edge and by `crate-p`'s edge: the vouched
/// one wins, so it loads once without consent.
#[tokio::test]
async fn a_diamond_with_one_vouched_edge_loads() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            add_registry_plugin(&ctx);
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            assert_eq!(synced(&mut ctx).await, "pt");
            let skills = ctx.workspace_root.clone().unwrap().join(".claude/skills");
            assert_eq!(installed(&skills, "t-guidance").len(), 1);
            Ok(())
        },
    )
    .await
    .unwrap();
}

async fn status(ctx: &TestContext) -> Vec<symposium::status_command::StatusEntry> {
    let deps = ctx.sym.workspace_deps(ctx.workspace_root.as_ref().unwrap());
    symposium::status_command::workspace_status(&ctx.sym, &deps)
        .await
        .unwrap()
}

/// A held-back chain target is a candidate in `status`, `pending_candidates`
/// and the `SessionStart` hint, each naming the crate that pulled it in; once
/// accepted it is active and no longer pending.
#[tokio::test]
async fn a_held_back_chain_target_is_surfaced_with_who_pulled_it_in() {
    use symposium::status_command::StatusState;
    use symposium_testlib::HookStep;

    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.sym.config.auto_update = symposium::config::AutoUpdate::Off;
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];

            let entries = status(&ctx).await;
            let t = entries.iter().find(|e| e.name == "crate-t").unwrap();
            assert_eq!(t.state, StatusState::Candidate, "{t:?}");
            assert!(t.root.contains("pulled in by `crate-p`"), "{t:?}");
            // `crate-u` is only reachable through `crate-t`, which is not loaded.
            let u = entries.iter().find(|e| e.name == "crate-u").unwrap();
            assert!(!u.root.contains("pulled in by"), "{u:?}");

            let deps = ctx.sym.workspace_deps(ctx.workspace_root.as_ref().unwrap());
            let notes = symposium::discovery::pending_candidate_notes(&ctx.sym, &deps).await;
            assert!(
                notes.contains(&("crate-t".to_string(), Some("crate-p".to_string()))),
                "{notes:?}"
            );

            let result = ctx
                .prompt_or_hook(
                    "hello",
                    &[HookStep::session_start()],
                    symposium::hook_schema::HookAgent::Claude,
                )
                .await?;
            let context = result
                .hooks
                .iter()
                .filter_map(|h| {
                    h.output
                        .get("additionalContext")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            h.output
                                .get("hookSpecificOutput")
                                .and_then(|o| o.get("additionalContext"))
                                .and_then(|v| v.as_str())
                        })
                })
                .next()
                .expect("session-start should produce additionalContext")
                .to_string();
            assert!(
                context.contains("crate-t (named by the plugin of `crate-p`)"),
                "{context}"
            );

            ctx.sym.config.plugins.used = vec![UseEntry::Global("crate-t".into())];
            let entries = status(&ctx).await;
            let t = entries.iter().find(|e| e.name == "crate-t").unwrap();
            assert_eq!(t.state, StatusState::Active, "{t:?}");
            let notes = symposium::discovery::pending_candidate_notes(&ctx.sym, &deps).await;
            assert!(notes.iter().all(|(n, _)| n != "crate-t"), "{notes:?}");
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// Answering "Enable" for a chained crate the workspace does not depend on
/// records a workspace-scoped `use` entry (`auto-enable` only covers
/// dependencies); one it does depend on keeps the `auto-enable` recording.
#[tokio::test]
async fn consent_records_use_for_a_non_dependency_and_auto_enable_for_a_dependency() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            let root = ctx.workspace_root.clone().unwrap();

            symposium::discovery::apply_consent_scoped(
                &mut ctx.sym,
                &["crate-t".to_string()],
                &[("crate-ghost".to_string(), root.clone())],
                &["crate-u".to_string()],
            )?;
            let plugins = &ctx.sym.config.plugins;
            assert_eq!(plugins.auto_enable, vec!["crate-t".to_string()]);
            assert_eq!(
                plugins.used,
                vec![UseEntry::Workspace {
                    name: "crate-ghost".into(),
                    workspace: root.clone()
                }]
            );
            assert_eq!(plugins.disable, vec!["crate-u".to_string()]);

            // Recording again, in another spelling, changes nothing.
            let before = std::fs::read_to_string(ctx.sym.config_dir().join("config.toml"))?;
            symposium::discovery::apply_consent_scoped(
                &mut ctx.sym,
                &["crate_t".to_string()],
                &[("crate_ghost".to_string(), root)],
                &["crate_u".to_string()],
            )?;
            let after = std::fs::read_to_string(ctx.sym.config_dir().join("config.toml"))?;
            assert_eq!(before, after);
            Ok(())
        },
    )
    .await
    .unwrap();
}

/// The prompt stays inert off a terminal even with a chain candidate pending.
#[tokio::test]
async fn the_prompt_stays_inert_for_chain_candidates() {
    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            let deps = ctx.sym.workspace_deps(ctx.workspace_root.as_ref().unwrap());
            symposium::discovery::prompt_for_consent(
                &mut ctx.sym,
                &deps,
                &symposium::output::Output::quiet(),
            )
            .await?;
            assert_eq!(
                ctx.sym.config.plugins.auto_enable,
                vec!["crate-p".to_string()]
            );
            assert!(ctx.sym.config.plugins.used.is_empty());
            assert!(ctx.sym.config.plugins.disable.is_empty());
            Ok(())
        },
    )
    .await
    .unwrap();
}
