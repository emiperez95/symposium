use super::*;

const GOLDEN: &str = "sha256:d52ea77032bf0b88b9a0ba259bf243d0ed061103c082ba79ebb237a0b290ea28";

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, bytes).unwrap();
}

const CARGO_TOML: &[u8] = b"[package]\nname = \"golden\"\nversion = \"1.0.0\"\n\n\
    [package.metadata.symposium]\ndepends-on = [\"serde\"]\npredicates = [\"env(FOO)\"]\n";

/// The golden fixture. Every byte is written explicitly (including the CRLFs)
/// so no platform converts line endings. The expected digest was computed by
/// an independent implementation of the scheme in the module docs.
fn fixture(root: &Path) {
    write(root, "Cargo.toml", CARGO_TOML);
    write(
        root,
        "Symposium.toml",
        b"name = \"golden\"\r\n[[skills]]\r\nsource.path = \"extra\"\r\n",
    );
    write(
        root,
        "extra/one/SKILL.md",
        b"---\nname: one\ndescription: first\n---\nbody one\n",
    );
    write(root, "extra/one/resources/data.bin", &[0, 1, 2, 255]);
    write(
        root,
        "skills/two/SKILL.md",
        b"---\nname: two\ndescription: second\n---\nbody two\r\n",
    );
    write(root, "skills/two/scripts/run.sh", b"echo hi\r\n");
    // Outside the payload.
    write(root, "src/lib.rs", b"pub fn code() {}\n");
    write(root, "README.md", b"not hashed\n");
    write(root, "other/notes.txt", b"not in a skill dir\n");
}

fn digest_of(root: &Path) -> String {
    payload_digest(root).unwrap().to_string()
}

fn tmp_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    tmp
}

#[test]
fn golden_digest() {
    let tmp = tmp_fixture();
    assert_eq!(digest_of(tmp.path()), GOLDEN);
}

#[test]
fn a_skill_resource_changes_the_digest() {
    let tmp = tmp_fixture();
    write(tmp.path(), "extra/one/resources/data.bin", &[0, 1, 2, 254]);
    assert_ne!(digest_of(tmp.path()), GOLDEN);

    let tmp = tmp_fixture();
    write(tmp.path(), "skills/two/scripts/run.sh", b"echo hi\n");
    assert_ne!(
        digest_of(tmp.path()),
        GOLDEN,
        "line endings are not normalized"
    );

    let tmp = tmp_fixture();
    write(tmp.path(), "skills/two/resources/new.txt", b"added");
    assert_ne!(digest_of(tmp.path()), GOLDEN, "an added file counts");
}

#[test]
fn code_and_unrelated_cargo_toml_keys_do_not_change_the_digest() {
    let tmp = tmp_fixture();
    write(tmp.path(), "src/lib.rs", b"pub fn other_code() {}\n");
    write(tmp.path(), "src/new.rs", b"fn x() {}");
    write(tmp.path(), "other/notes.txt", b"changed");
    let mut cargo = CARGO_TOML.to_vec();
    cargo.extend_from_slice(b"\n[dependencies]\nserde = \"1\"\n");
    let cargo = String::from_utf8(cargo)
        .unwrap()
        .replace("version = \"1.0.0\"", "version = \"2.3.4\"");
    write(tmp.path(), "Cargo.toml", cargo.as_bytes());
    assert_eq!(digest_of(tmp.path()), GOLDEN);
}

#[test]
fn metadata_key_order_and_formatting_do_not_matter() {
    let tmp = tmp_fixture();
    write(
        tmp.path(),
        "Cargo.toml",
        b"[package]\nname = \"golden\"\nversion = \"1.0.0\"\n\n\
          [package.metadata.symposium]\n\n  predicates = [ \"env(FOO)\" ]\n\
          depends-on   = [\"serde\"]\n",
    );
    assert_eq!(digest_of(tmp.path()), GOLDEN);
}

#[test]
fn changing_the_metadata_changes_the_digest() {
    let tmp = tmp_fixture();
    write(
        tmp.path(),
        "Cargo.toml",
        b"[package]\nname = \"golden\"\n[package.metadata.symposium]\n\
          depends-on = [\"tokio\"]\npredicates = [\"env(FOO)\"]\n",
    );
    assert_ne!(digest_of(tmp.path()), GOLDEN);
}

#[test]
fn renaming_a_file_changes_the_digest_but_creation_order_does_not() {
    let tmp = tmp_fixture();
    std::fs::rename(
        tmp.path().join("skills/two/scripts/run.sh"),
        tmp.path().join("skills/two/scripts/go.sh"),
    )
    .unwrap();
    assert_ne!(digest_of(tmp.path()), GOLDEN);

    // The same files written in the opposite order hash the same.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "skills/two/scripts/run.sh", b"echo hi\r\n");
    write(
        root,
        "skills/two/SKILL.md",
        b"---\nname: two\ndescription: second\n---\nbody two\r\n",
    );
    write(root, "extra/one/resources/data.bin", &[0, 1, 2, 255]);
    write(
        root,
        "extra/one/SKILL.md",
        b"---\nname: one\ndescription: first\n---\nbody one\n",
    );
    write(
        root,
        "Symposium.toml",
        b"name = \"golden\"\r\n[[skills]]\r\nsource.path = \"extra\"\r\n",
    );
    write(root, "Cargo.toml", CARGO_TOML);
    assert_eq!(digest_of(root), GOLDEN);
}

#[test]
fn a_crate_with_no_manifest_hashes_its_default_skills_directory() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "skills/guide/SKILL.md",
        b"---\nname: guide\n---\n",
    );
    let with_skill = payload_digest(tmp.path()).unwrap();
    write(
        tmp.path(),
        "skills/guide/SKILL.md",
        b"---\nname: guide2\n---\n",
    );
    assert_ne!(payload_digest(tmp.path()).unwrap(), with_skill);

    // Opting out of the default group takes the skills out of the payload.
    write(
        tmp.path(),
        "Symposium.toml",
        b"[defaults]\nskills = false\n",
    );
    let opted_out = payload_digest(tmp.path()).unwrap();
    write(
        tmp.path(),
        "skills/guide/SKILL.md",
        b"---\nname: guide3\n---\n",
    );
    assert_eq!(payload_digest(tmp.path()).unwrap(), opted_out);
}

#[test]
fn a_file_reached_through_two_groups_counts_once() {
    let one = tempfile::tempdir().unwrap();
    write(
        one.path(),
        "Symposium.toml",
        b"[[skills]]\nsource.path = \"skills\"\n",
    );
    write(one.path(), "skills/a/SKILL.md", b"---\nname: a\n---\n");

    // `skills` is already the default group, so this overlaps it exactly.
    let two = tempfile::tempdir().unwrap();
    write(
        two.path(),
        "Symposium.toml",
        b"[[skills]]\nsource.path = \"skills\"\n[[skills]]\nsource.path = \"skills/a\"\n",
    );
    write(two.path(), "skills/a/SKILL.md", b"---\nname: a\n---\n");
    let a = payload_digest(one.path()).unwrap();
    let b = payload_digest(two.path()).unwrap();
    // The manifests differ, so the digests do; what matters is that the
    // overlapping group neither errors nor double-counts the shared file.
    assert_ne!(a, b);
    let entries = payload_entries(two.path()).unwrap();
    assert!(entries.contains_key("skills/a/SKILL.md"));
    assert_eq!(entries.len(), 2, "{:?}", entries.keys().collect::<Vec<_>>());
}

#[test]
fn a_git_skill_group_is_ineligible() {
    let tmp = tmp_fixture();
    write(
        tmp.path(),
        "Symposium.toml",
        b"[[skills]]\nsource.git = \"https://github.com/example/skills\"\n",
    );
    assert_eq!(
        payload_digest(tmp.path()).unwrap_err(),
        Ineligible::GitSkillSource("https://github.com/example/skills".into())
    );
}

#[test]
fn a_skill_path_leaving_the_crate_is_ineligible() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("crate");
    write(
        &root,
        "Symposium.toml",
        b"[[skills]]\nsource.path = \"../shared\"\n",
    );
    write(tmp.path(), "shared/s/SKILL.md", b"---\nname: s\n---\n");
    assert_eq!(
        payload_digest(&root).unwrap_err(),
        Ineligible::EscapesCrate("../shared".into())
    );
}

#[test]
fn an_oversized_payload_is_ineligible() {
    let tmp = tmp_fixture();
    write(
        tmp.path(),
        "skills/two/big.bin",
        &vec![0u8; MAX_PAYLOAD_BYTES as usize],
    );
    assert_eq!(
        payload_digest(tmp.path()).unwrap_err(),
        Ineligible::TooLarge
    );

    // Just under the cap is fine.
    let tmp = tmp_fixture();
    write(
        tmp.path(),
        "skills/two/big.bin",
        &vec![0u8; MAX_PAYLOAD_BYTES as usize - 1024],
    );
    assert!(payload_digest(tmp.path()).is_ok());
}

#[test]
fn an_unusable_manifest_or_root_is_ineligible() {
    let tmp = tmp_fixture();
    write(tmp.path(), "Symposium.toml", b"[[skills]]\n");
    assert!(matches!(
        payload_digest(tmp.path()).unwrap_err(),
        Ineligible::Manifest(_)
    ));
    assert!(matches!(
        payload_digest(&tmp.path().join("missing")).unwrap_err(),
        Ineligible::NotADirectory(_)
    ));
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    /// Permission tests mean nothing as root, which reads everything.
    fn running_as_root(tmp: &Path) -> bool {
        let probe = tmp.join("probe");
        std::fs::create_dir(&probe).unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&probe).is_ok();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o700)).unwrap();
        readable
    }

    #[test]
    fn a_dangling_group_directory_is_ineligible_not_missing() {
        let tmp = tmp_fixture();
        symlink("nowhere", tmp.path().join("extra-link")).unwrap();
        write(
            tmp.path(),
            "Symposium.toml",
            b"name = \"golden\"\n[[skills]]\nsource.path = \"extra-link\"\n",
        );
        assert_eq!(
            payload_digest(tmp.path()).unwrap_err(),
            Ineligible::BrokenLink("extra-link".into())
        );
    }

    #[test]
    fn a_dangling_manifest_link_is_ineligible_not_absent() {
        let tmp = tmp_fixture();
        std::fs::remove_file(tmp.path().join("Symposium.toml")).unwrap();
        symlink("nowhere", tmp.path().join("Symposium.toml")).unwrap();
        assert_eq!(
            payload_digest(tmp.path()).unwrap_err(),
            Ineligible::BrokenLink("Symposium.toml".into())
        );
    }

    #[test]
    fn an_unreadable_manifest_is_ineligible() {
        let tmp = tmp_fixture();
        if running_as_root(tmp.path()) {
            return;
        }
        for file in ["Symposium.toml", "Cargo.toml"] {
            let path = tmp.path().join(file);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            let err = payload_digest(tmp.path()).unwrap_err();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                matches!(err, Ineligible::Unreadable(_) | Ineligible::Manifest(_)),
                "{file}: {err:?}"
            );
        }
        assert!(payload_digest(tmp.path()).is_ok());
    }

    #[test]
    fn an_unreadable_skill_directory_is_ineligible() {
        let tmp = tmp_fixture();
        if running_as_root(tmp.path()) {
            return;
        }
        // Unlistable group directory, and an unlistable directory inside one.
        for dir in ["skills", "skills/two/scripts"] {
            let path = tmp.path().join(dir);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            let err = payload_digest(tmp.path()).unwrap_err();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(matches!(err, Ineligible::Unreadable(_)), "{dir}: {err:?}");
        }
        assert_eq!(digest_of(tmp.path()), GOLDEN);
    }

    #[test]
    fn a_broken_link_beside_the_skills_is_ineligible() {
        // Discovery ignores it, but it could be repointed at a directory of
        // skills the digest never saw.
        let tmp = tmp_fixture();
        symlink("nowhere", tmp.path().join("skills/maybe-skills")).unwrap();
        assert!(matches!(
            payload_digest(tmp.path()).unwrap_err(),
            Ineligible::BrokenLink(_)
        ));
    }

    #[test]
    fn a_broken_symlink_is_ineligible() {
        let tmp = tmp_fixture();
        symlink("nowhere", tmp.path().join("skills/two/dangling")).unwrap();
        assert_eq!(
            payload_digest(tmp.path()).unwrap_err(),
            Ineligible::BrokenLink("skills/two/dangling".into())
        );
    }

    #[test]
    fn a_directory_loop_is_ineligible() {
        let tmp = tmp_fixture();
        symlink("..", tmp.path().join("skills/two/back")).unwrap();
        assert_eq!(
            payload_digest(tmp.path()).unwrap_err(),
            Ineligible::LinkLoop("skills/two/back".into())
        );
    }

    /// A linked file reads as the file it names, under the link's own path.
    #[test]
    fn a_linked_file_is_hashed_as_content_under_the_links_name() {
        let tmp = tmp_fixture();
        write(tmp.path(), "shared/data.bin", &[9, 9]);
        symlink(
            "../../shared/data.bin",
            tmp.path().join("skills/two/linked.bin"),
        )
        .unwrap();
        let linked = digest_of(tmp.path());
        assert_ne!(linked, GOLDEN);

        let copy = tmp_fixture();
        write(copy.path(), "skills/two/linked.bin", &[9, 9]);
        assert_eq!(digest_of(copy.path()), linked);
    }
}

#[test]
fn canonical_json_sorts_keys_and_stringifies_datetimes() {
    let value: toml::Value = toml::from_str(
        "z = 1\na = [true, 1.5, \"x\\\"y\"]\nwhen = 2024-01-02T03:04:05Z\n[m]\nb = 1\na = 2\n",
    )
    .unwrap();
    let mut out = String::new();
    canonical_json(&value, &mut out);
    assert_eq!(
        out,
        r#"{"a":[true,1.5,"x\"y"],"m":{"a":2,"b":1},"when":"2024-01-02T03:04:05Z","z":1}"#
    );
}
