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
            Ok(())
        },
    )
    .await
    .unwrap();
}
