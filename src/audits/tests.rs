use super::*;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn digest(s: &str) -> Digest {
    s.parse().unwrap()
}

fn parse(text: &str) -> (Option<CrateAudits>, Vec<String>) {
    parse_file("reg", "widget-lib", text)
}

fn write_audits(registry: &Path, file: &str, text: &str) {
    std::fs::create_dir_all(registry.join("audits")).unwrap();
    std::fs::write(registry.join("audits").join(file), text).unwrap();
}

#[test]
fn file_name_is_lowercase_underscored_and_confined() {
    assert_eq!(audit_file_name("widget-lib").unwrap(), "widget_lib.toml");
    assert_eq!(audit_file_name("Widget_Lib").unwrap(), "widget_lib.toml");
    for bad in ["", "../x", "a/b", "a.b", "a b"] {
        assert_eq!(audit_file_name(bad), None, "{bad}");
    }
}

#[test]
fn a_well_formed_file_yields_audits_and_revocations() {
    let (audits, warnings) = parse(&format!(
        r#"
format = 1
crate = "widget-lib"

[[audit]]
digest = "{A}"
versions = ["1.4.2"]

[[revoked]]
versions = ">=1.3.0, <1.4.2"
reason = "remote-exec"

[[revoked]]
digest = "{B}"
reason = "exfiltration"
"#
    ));
    assert!(warnings.is_empty(), "{warnings:?}");
    let audits = audits.unwrap();
    assert_eq!(audits.audited_by(&digest(A)), Some("reg"));
    assert_eq!(audits.audited_by(&digest(B)), None);
    assert!(audits.has_digest_revocation());

    let v = |s: &str| semver::Version::parse(s).unwrap();
    let hit = audits.revocation(Some(&v("1.3.5")), None).unwrap();
    assert_eq!(hit.reason, RevokeReason::RemoteExec);
    assert_eq!(hit.registry, "reg");
    assert!(audits.revocation(Some(&v("1.4.2")), None).is_none());
    assert_eq!(
        audits.revocation(None, Some(&digest(B))).unwrap().reason,
        RevokeReason::Exfiltration
    );
    // A version that is not semver (None) never matches a requirement.
    assert!(audits.revocation(None, None).is_none());
}

#[test]
fn an_unknown_format_ignores_the_whole_file_with_a_warning() {
    let (audits, warnings) = parse(&format!(
        "format = 2\ncrate = \"widget-lib\"\n[[audit]]\ndigest = \"{A}\"\n\
         [[revoked]]\nversions = \"*\"\nreason = \"report\"\n"
    ));
    assert!(audits.is_none());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unsupported format 2"), "{warnings:?}");

    let (audits, warnings) = parse("crate = \"widget-lib\"\n");
    assert!(audits.is_none());
    assert!(warnings[0].contains("(missing)"), "{warnings:?}");
}

#[test]
fn a_revoked_key_that_is_not_an_array_warns_and_drops_it() {
    let (audits, warnings) = parse(&format!(
        "format = 1\ncrate = \"widget-lib\"\n\n[revoked]\nversions = \"<2.0.0\"\n\n[[audit]]\ndigest = \"{A}\"\n"
    ));
    let audits = audits.unwrap();
    assert!(audits.revoked.is_empty());
    assert_eq!(audits.audited.len(), 1, "the rest of the file is kept");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("ignoring `revoked`") && warnings[0].contains("[[revoked]]"),
        "{warnings:?}"
    );

    let (audits, warnings) =
        parse("format = 1\ncrate = \"widget-lib\"\nrevoked = \"x\"\naudit = 3\n");
    assert!(audits.unwrap().is_empty());
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("`audit`") || warnings[1].contains("`audit`"));
}

#[test]
fn malformed_entries_are_dropped_and_the_rest_kept() {
    let (audits, warnings) = parse(&format!(
        r#"
format = 1
crate = "widget-lib"

[[audit]]
digest = "not-a-digest"

[[audit]]
digest = "{A}"

[[revoked]]
versions = "not a requirement"

[[revoked]]
versions = "<1.0.0"
digest = "{B}"

[[revoked]]
reason = "report"

[[revoked]]
digest = "sha256:short"

[[revoked]]
versions = "<2.0.0"
reason = "advisory"
"#
    ));
    let audits = audits.unwrap();
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert_eq!(audits.audited.len(), 1);
    assert_eq!(audits.revoked.len(), 1);
    assert_eq!(audits.revoked[0].reason, RevokeReason::Advisory);
}

#[test]
fn a_file_for_another_crate_is_ignored() {
    let (audits, warnings) = parse("format = 1\ncrate = \"other\"\n");
    assert!(audits.is_none());
    assert!(warnings[0].contains("`crate` is not"), "{warnings:?}");
    // Spelling differences within one crate are fine.
    let (audits, _) = parse("format = 1\ncrate = \"Widget_Lib\"\n");
    assert!(audits.unwrap().is_empty());
}

#[test]
fn invalid_toml_is_ignored() {
    let (audits, warnings) = parse("format = [");
    assert!(audits.is_none());
    assert!(warnings[0].contains("not valid TOML"), "{warnings:?}");
}

#[test]
fn an_unknown_reason_is_kept_as_unknown() {
    let (audits, _) = parse(
        "format = 1\ncrate = \"widget-lib\"\n[[revoked]]\nversions = \"*\"\nreason = \"newer\"\n",
    );
    let hit = audits
        .unwrap()
        .revocation(Some(&semver::Version::new(1, 0, 0)), None)
        .unwrap();
    assert_eq!(hit.reason, RevokeReason::Unknown);
}

/// The lookup reads `audits/<name>.toml` from each source, merges them, and
/// reports which registry an audit came from.
#[test]
fn the_index_merges_registries_and_names_the_matching_one() {
    let tmp = tempfile::tempdir().unwrap();
    let one = tmp.path().join("one");
    let two = tmp.path().join("two");
    write_audits(
        &one,
        "widget_lib.toml",
        &format!("format = 1\ncrate = \"widget-lib\"\n[[audit]]\ndigest = \"{A}\"\n"),
    );
    write_audits(
        &two,
        "widget_lib.toml",
        &format!(
            "format = 1\ncrate = \"widget-lib\"\n[[audit]]\ndigest = \"{B}\"\n\
             [[revoked]]\nversions = \"<1.0.0\"\nreason = \"report\"\n"
        ),
    );
    let index = AuditIndex::new(vec![
        AuditSource {
            registry: "one".into(),
            dir: one,
        },
        AuditSource {
            registry: "two".into(),
            dir: two,
        },
    ]);

    assert_eq!(
        index.audited_by("widget-lib", &digest(A)).as_deref(),
        Some("one")
    );
    assert_eq!(
        index.audited_by("widget_lib", &digest(B)).as_deref(),
        Some("two")
    );
    assert_eq!(
        index.audited_by("widget-lib", &digest(&format!("sha256:{}", "c".repeat(64)))),
        None
    );
    assert_eq!(index.audited_by("other", &digest(A)), None);

    let hit = index
        .for_crate("widget-lib")
        .revocation(Some(&semver::Version::new(0, 9, 0)), None)
        .unwrap();
    assert_eq!(hit.registry, "two");
}

/// A missing registry directory or audits directory is simply no audits.
#[test]
fn missing_files_are_no_audits() {
    let tmp = tempfile::tempdir().unwrap();
    let index = AuditIndex::new(vec![AuditSource {
        registry: "gone".into(),
        dir: tmp.path().join("nope"),
    }]);
    assert!(index.for_crate("widget-lib").is_empty());
    assert!(AuditIndex::default().for_crate("widget-lib").is_empty());
}

/// `AuditIndex::revocation` matches a cargo package by version or by the
/// digest of the payload at its root, and only a cargo package.
#[test]
fn revocation_matches_a_cargo_package_by_version_or_payload() {
    use symposium_sdk::pm::CARGO_PM;

    let tmp = tempfile::tempdir().unwrap();
    let krate = tmp.path().join("crate");
    std::fs::create_dir_all(krate.join("skills/s")).unwrap();
    std::fs::write(krate.join("skills/s/SKILL.md"), "---\nname: s\n---\n").unwrap();
    let payload = digest::payload_digest(&krate).unwrap();

    let registry = tmp.path().join("registry");
    write_audits(
        &registry,
        "widget_lib.toml",
        &format!(
            "format = 1\ncrate = \"widget-lib\"\n\
             [[revoked]]\nversions = \">=2.0.0, <2.1.0\"\nreason = \"advisory\"\n\
             [[revoked]]\ndigest = \"{payload}\"\nreason = \"injection-bait\"\n"
        ),
    );
    let index = AuditIndex::new(vec![AuditSource {
        registry: "reg".into(),
        dir: registry,
    }]);
    let id = |version: &str| PackageId::new(CARGO_PM, "widget-lib", version);

    // By payload, whatever the version (including one that is not semver).
    let by_payload = index.revocation(&id("9.9.9"), &krate).unwrap();
    assert_eq!(by_payload.reason, RevokeReason::InjectionBait);
    assert!(index.revocation(&id("*"), &krate).is_some());

    // By version, when the payload differs.
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let hit = index.revocation(&id("2.0.5"), &other).unwrap();
    assert_eq!(
        (hit.registry.as_str(), hit.reason),
        ("reg", RevokeReason::Advisory)
    );
    assert!(index.revocation(&id("2.1.0"), &other).is_none());
    assert!(index.revocation(&id("not-semver"), &other).is_none());

    // A payload with no digest cannot match a digest revocation, but a version
    // revocation still applies.
    let missing = tmp.path().join("missing");
    assert!(index.revocation(&id("2.1.0"), &missing).is_none());
    assert!(index.revocation(&id("2.0.1"), &missing).is_some());

    // Only cargo packages are covered.
    let registry_plugin = PackageId::new("reg", "widget-lib", "2.0.5");
    assert!(index.revocation(&registry_plugin, &other).is_none());
    // A crate nobody audited costs nothing and matches nothing.
    let unaudited = PackageId::new(CARGO_PM, "gadget", "1.0.0");
    assert!(index.revocation(&unaudited, &krate).is_none());
}

#[test]
fn lookup_distinguishes_audited_differing_and_unaudited() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("src-widget");
    std::fs::create_dir_all(crate_dir.join("skills/s")).unwrap();
    std::fs::write(crate_dir.join("skills/s/SKILL.md"), "hello").unwrap();
    let payload = digest::payload_digest(&crate_dir).unwrap();

    let registry = tmp.path().join("reg");
    let index = AuditIndex::new(vec![AuditSource {
        registry: "reg".into(),
        dir: registry.clone(),
    }]);
    let id = PackageId::new(CARGO_PM, "widget-lib", "1.0.0");
    assert!(!index.any_audits_dir());
    assert_eq!(index.lookup(&id, &crate_dir), AuditLookup::Unaudited);

    write_audits(
        &registry,
        "widget_lib.toml",
        &format!("format = 1\ncrate = \"widget-lib\"\n[[audit]]\ndigest = \"{payload}\"\n"),
    );
    assert!(index.any_audits_dir());
    assert!(index.for_crate("widget_lib").has_audits());
    assert_eq!(
        index.lookup(&id, &crate_dir),
        AuditLookup::Audited("reg".into())
    );
    // Another package manager's package is never audited.
    let other = PackageId::new("reg", "widget-lib", "1.0.0");
    assert_eq!(index.lookup(&other, &crate_dir), AuditLookup::Unaudited);

    std::fs::write(crate_dir.join("skills/s/SKILL.md"), "changed").unwrap();
    assert_eq!(index.lookup(&id, &crate_dir), AuditLookup::PayloadDiffers);
}

#[test]
fn a_crate_with_audits_but_an_ineligible_payload_is_ineligible_not_unaudited() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("src-widget");
    std::fs::create_dir_all(crate_dir.join("skills/s")).unwrap();
    std::fs::write(crate_dir.join("skills/s/SKILL.md"), "hello").unwrap();

    let registry = tmp.path().join("reg");
    let index = AuditIndex::new(vec![AuditSource {
        registry: "reg".into(),
        dir: registry.clone(),
    }]);
    write_audits(
        &registry,
        "widget_lib.toml",
        &format!(
            "format = 1\ncrate = \"widget-lib\"\n[[audit]]\ndigest = \"{}\"\n",
            digest(A)
        ),
    );
    std::fs::write(
        crate_dir.join("Symposium.toml"),
        "[[skills]]\nsource.git = \"https://github.com/example/skills\"\n",
    )
    .unwrap();
    let id = PackageId::new(CARGO_PM, "widget-lib", "1.0.0");
    assert_eq!(index.lookup(&id, &crate_dir), AuditLookup::Ineligible);
    // The same payload under a crate nobody audited is simply unaudited.
    let other = PackageId::new(CARGO_PM, "other-lib", "1.0.0");
    assert_eq!(index.lookup(&other, &crate_dir), AuditLookup::Unaudited);
}
