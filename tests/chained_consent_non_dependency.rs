//! A `[[plugins]]` chain target that is not a workspace dependency, reached
//! through cargo's registry cache (`CARGO_HOME`).
//!
//! `CARGO_HOME` is process-global, so this lives in its own test binary (one
//! test, one process), like `tests/audits_non_dependency.rs`. Fixture
//! `chain-consent-nondep0`: the host depends on `crate-p` only, and `crate-p`
//! chains to `crate-z`, which is cached under a fake `CARGO_HOME`.

use std::path::{Path, PathBuf};

use symposium::audits::digest::payload_digest;
use symposium::config::AutoAccept;
use symposium_testlib::{TestContext, TestMode, with_fixture};

/// Put `name-version` into a fake cargo registry cache the way cargo leaves an
/// extracted crate, returning its directory.
fn cache_crate(cargo_home: &Path, name: &str, version: &str, skill: &str) -> PathBuf {
    let dir = cargo_home
        .join("registry/src/index.crates.io-1949cf8c6b5b557f")
        .join(format!("{name}-{version}"));
    std::fs::create_dir_all(dir.join(format!("skills/{skill}"))).unwrap();
    std::fs::write(dir.join(".cargo-ok"), "{\"v\":1}").unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("skills/{skill}/SKILL.md")),
        format!("---\nname: {skill}\ndescription: guidance\n---\nbody\n"),
    )
    .unwrap();
    // A subcommand running a binary that is always on PATH under `cargo test`.
    std::fs::write(
        dir.join("Symposium.toml"),
        "[[installations]]\nname = \"x\"\nexecutable = \"rustc\"\n\n\
         [subcommand.ztool]\ndescription = \"z\"\ncommand = \"x\"\n",
    )
    .unwrap();
    dir
}

fn write_audit(ctx: &TestContext, crate_name: &str, digest: impl std::fmt::Display) {
    let dir = ctx.sym.config_dir().join("plugins/audits");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{}.toml", crate_name.replace('-', "_"))),
        format!("format = 1\ncrate = \"{crate_name}\"\n[[audit]]\ndigest = \"{digest}\"\n"),
    )
    .unwrap();
}

/// Whether crate-z's subcommand dispatches. Dispatch resolves the active plugin
/// set at `UpdateLevel::None` (cargo's cache only), as the hook path does.
async fn z_dispatches(ctx: &mut TestContext) -> bool {
    match ctx.symposium(&["ztool", "--version"]).await {
        Ok(out) => {
            assert!(out.contains("rustc"), "{out}");
            true
        }
        Err(_) => false,
    }
}

#[tokio::test]
async fn a_chain_target_that_is_not_a_dependency() {
    let cargo_home = tempfile::tempdir().unwrap();
    let z_dir = cache_crate(cargo_home.path(), "crate-z", "1.0.0", "z-guidance");
    // SAFETY: this binary holds one test and nothing else touches the
    // environment before the harness reads it.
    unsafe { std::env::set_var("CARGO_HOME", cargo_home.path()) };

    with_fixture(
        TestMode::SimulationOnly,
        &["chain-consent-nondep0"],
        async |mut ctx| {
            ctx.symposium(&["init", "--add-agent", "claude"]).await?;
            ctx.sym.config.plugins.auto_enable = vec!["crate-p".into()];
            ctx.sym.config.plugins.auto_accept = AutoAccept::Audit;

            // Nothing accepts crate-z yet.
            assert!(!z_dispatches(&mut ctx).await);

            // Both targets are listed as candidates naming the package that pulled
            // them in. The cached one is described; `crate-w`, in no cache, is
            // still pending but undescribed.
            let root = ctx.workspace_root.clone().unwrap();
            let deps = ctx.sym.workspace_deps(&root);
            let found = symposium::discovery::discover_for(&ctx.sym, &deps).await;
            let candidate = |name: &str| {
                found
                    .candidates
                    .iter()
                    .find(|d| d.id.name == name)
                    .unwrap_or_else(|| panic!("no candidate `{name}` in {:?}", found.candidates))
            };
            let z = candidate("crate-z");
            assert_eq!(z.pulled_in_by.as_ref().unwrap().name, "crate-p");
            assert!(z.description.as_deref().unwrap().contains("skill"), "{z:?}");
            let w = candidate("crate-w");
            assert_eq!(w.pulled_in_by.as_ref().unwrap().name, "crate-p");
            assert_eq!(w.description, None, "{w:?}");
            let entries = symposium::status_command::workspace_status(&ctx.sym, &deps).await?;
            let z = entries.iter().find(|e| e.name == "crate-z").unwrap();
            assert!(z.root.contains("pulled in by `crate-p`"), "{z:?}");

            // An audit of a different payload than the one that loads does not
            // accept it: the decision is made on the loaded release.
            let other = cache_crate(
                cargo_home.path().join("elsewhere").as_path(),
                "crate-z",
                "0.9.0",
                "other-guidance",
            );
            write_audit(&ctx, "crate-z", payload_digest(&other).unwrap());
            assert!(!z_dispatches(&mut ctx).await);

            // An audit of the payload that loads does.
            write_audit(&ctx, "crate-z", payload_digest(&z_dir).unwrap());
            assert!(z_dispatches(&mut ctx).await);

            // `auto-accept = "none"` never lets an audit decide.
            ctx.sym.config.plugins.auto_accept = AutoAccept::None;
            assert!(!z_dispatches(&mut ctx).await);
            ctx.sym.config.plugins.auto_accept = AutoAccept::Audit;
            assert!(z_dispatches(&mut ctx).await);

            // Disabled: one `declined` line (from the config entry), not pending.
            let root = ctx.workspace_root.clone().unwrap();
            let deps = ctx.sym.workspace_deps(&root);
            ctx.sym.config.plugins.disable = vec!["crate-z".into()];
            assert!(!z_dispatches(&mut ctx).await);
            let entries = symposium::status_command::workspace_status(&ctx.sym, &deps).await?;
            let z: Vec<_> = entries.iter().filter(|e| e.name == "crate-z").collect();
            assert_eq!(z.len(), 1, "{entries:?}");
            assert_eq!(z[0].state, symposium::status_command::StatusState::Declined);
            let notes = symposium::discovery::pending_candidate_notes(&ctx.sym, &deps).await;
            assert!(notes.iter().all(|(n, _)| n != "crate-z"), "{notes:?}");
            ctx.sym.config.plugins.disable = vec![];

            // Revoked: refused and, being reached only through a chain and not
            // a dependency, not listed at all (as the status reference says).
            let audits = ctx.sym.config_dir().join("plugins/audits");
            std::fs::write(
                audits.join("crate_z.toml"),
                "format = 1\ncrate = \"crate-z\"\n[[revoked]]\nversions = \"*\"\nreason = \"exfiltration\"\n",
            )?;
            assert!(!z_dispatches(&mut ctx).await);
            let entries = symposium::status_command::workspace_status(&ctx.sym, &deps).await?;
            assert!(entries.iter().all(|e| e.name != "crate-z"), "{entries:?}");
            let notes = symposium::discovery::pending_candidate_notes(&ctx.sym, &deps).await;
            assert!(notes.iter().all(|(n, _)| n != "crate-z"), "{notes:?}");
            Ok(())
        },
    )
    .await
    .unwrap();
}
