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
