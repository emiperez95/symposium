//! A revocation applies to a `use`d crate the workspace does not depend on.
//!
//! Such a crate is loaded from cargo's registry cache, found through
//! `CARGO_HOME`. That is process-global, so this lives in its own test binary
//! (one test, one process) instead of beside the tests in `tests/audits.rs`,
//! which run cargo concurrently.

use std::path::Path;

use symposium::config::UseEntry;
use symposium::status_command::{StatusEntry, StatusState};
use symposium_testlib::{TestMode, with_fixture};

/// Put `name-version` into a fake cargo registry cache the way cargo leaves an
/// extracted crate: `registry/src/index.crates.io-*/<name>-<version>/` with a
/// `.cargo-ok` marker.
fn cache_crate(cargo_home: &Path, name: &str, version: &str, skill: &str) {
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
         [subcommand.xtool]\ndescription = \"x\"\ncommand = \"x\"\n",
    )
    .unwrap();
}

fn write_audits(registry: &Path, crate_file: &str, text: &str) {
    let dir = registry.join("audits");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(crate_file), text).unwrap();
}

/// Whether crate-x's subcommand dispatches. Subcommand dispatch resolves the
/// active plugin set with `UpdateLevel::None`, cargo's cache only, exactly like
/// the hook path.
///
/// (`sync` would be the more direct probe, but `sync::sync` resolves the active
/// set with `UpdateLevel::Check`, which for a crate the workspace does not
/// depend on means a probe project and the network, something a test cannot
/// rely on. The activation logic, including the revocation check, is the same
/// code.)
async fn crate_x_dispatches(ctx: &mut symposium_testlib::TestContext) -> bool {
    match ctx.symposium(&["xtool", "--version"]).await {
        Ok(out) => {
            assert!(out.contains("rustc"), "{out}");
            true
        }
        Err(_) => false,
    }
}

fn find<'a>(entries: &'a [StatusEntry], name: &str) -> &'a StatusEntry {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no status entry for `{name}` in {entries:?}"))
}

#[tokio::test]
async fn a_used_crate_that_is_not_a_dependency_can_be_revoked() {
    let cargo_home = tempfile::tempdir().unwrap();
    // Cached: crate-x. Not cached anywhere: crate-y.
    cache_crate(cargo_home.path(), "crate-x", "2.0.0", "x-guidance");
    // SAFETY: this binary holds one test and nothing else touches the
    // environment before the harness reads it.
    unsafe { std::env::set_var("CARGO_HOME", cargo_home.path()) };

    with_fixture(TestMode::SimulationOnly, &["audits0"], async |mut ctx| {
        ctx.symposium(&["init", "--add-agent", "claude"]).await?;
        let workspace = ctx.workspace_root.clone().unwrap();
        ctx.sym.config.plugins.used = vec![
            UseEntry::Global("crate-x".into()),
            UseEntry::Global("crate-y".into()),
        ];
        let registry = ctx.tempdir.join("company-audits");
        let status = async |ctx: &symposium_testlib::TestContext| {
            let deps = ctx.sym.workspace_deps(&workspace);
            symposium::status_command::workspace_status(&ctx.sym, &deps)
                .await
                .unwrap()
        };

        // Without a revocation the cached crate loads.
        assert!(crate_x_dispatches(&mut ctx).await, "cached and not revoked");
        let entries = status(&ctx).await;
        assert_eq!(find(&entries, "crate-x").state, StatusState::Active);

        // Revoke both. crate-x is withheld and shows version and registry;
        // crate-y is in no cache, so its release is unknown and it
        // falls through to the ordinary line, with no error.
        for krate in ["crate_x", "crate_y"] {
            write_audits(
                &registry,
                &format!("{krate}.toml"),
                &format!(
                    "format = 1\ncrate = \"{}\"\n[[revoked]]\nversions = \"*\"\nreason = \"remote-exec\"\n",
                    krate.replace('_', "-")
                ),
            );
        }
        assert!(!crate_x_dispatches(&mut ctx).await, "a revoked crate is withheld");
        let entries = status(&ctx).await;
        let x = find(&entries, "crate-x");
        assert_eq!(x.state, StatusState::Revoked, "{x:?}");
        assert_eq!(x.version.as_deref(), Some("2.0.0"));
        assert!(x.root.contains("`company`"), "{x:?}");
        assert!(x.root.contains("fetch and run remote code"), "{x:?}");

        let y = find(&entries, "crate-y");
        assert_eq!(y.state, StatusState::Dormant, "{y:?}");
        assert!(y.root.contains("cached crate"), "{y:?}");
        assert_eq!(y.version, None);
        Ok(())
    })
    .await
    .unwrap();
}
