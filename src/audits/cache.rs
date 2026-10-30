//! A small on-disk cache of payload digests.
//!
//! Hashing a crate's payload on every resolution would repeat the same work
//! for the same bytes, so a digest is remembered under
//! `<cache-dir>/audit-digests/`, keyed on the exact package (name and version)
//! and on the digest scheme's version ([`DOMAIN`]), so changing the scheme
//! invalidates every entry.
//!
//! Only **immutable** sources are cached: a crate cargo extracted from the
//! crates.io registry, whose content cannot differ for a given name and
//! version. A path dependency, a workspace member and a git checkout can
//! change under an unchanged version, so those are always hashed.
//!
//! That immutability is an assumption, not a check: a hit is trusted without
//! re-reading the tree, so it holds only while nothing edits the registry's
//! extracted sources (`~/.cargo/registry/src`) in place.
//!
//! The cache is an optimisation and never an authority: any failure to read,
//! parse or write an entry falls back to hashing, silently.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use symposium_sdk::audits::Digest;
use symposium_sdk::pm::{CARGO_PM, PackageId};

use super::digest::{DOMAIN, Ineligible, payload_digest};

/// Directory name under the Symposium cache directory.
const CACHE_DIR: &str = "audit-digests";

/// Directory-name prefixes cargo gives the crates.io registry's extracted
/// sources (`registry/src/<prefix>...`): the sparse index, and the legacy git
/// index.
const CRATES_IO_SOURCES: [&str; 2] = ["index.crates.io-", "github.com-1ecc6299db9ec823"];

/// Whether `root` is the extracted source of `id` from the crates.io registry:
/// `.../registry/src/<crates.io source>/<name>-<version>`.
pub(super) fn is_immutable(id: &PackageId, root: &Path) -> bool {
    if id.pm != CARGO_PM {
        return false;
    }
    let mut parts = root.components().rev().map(|c| c.as_os_str().to_str());
    let (Some(Some(dir)), Some(Some(source)), Some(Some("src")), Some(Some("registry"))) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    dir == format!("{}-{}", id.name, id.version)
        && CRATES_IO_SOURCES.iter().any(|p| source.starts_with(p))
}

fn entry_path(cache: &Path, id: &PackageId) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(id.name.as_bytes());
    hasher.update([0]);
    hasher.update(id.version.as_bytes());
    let key: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    cache.join(CACHE_DIR).join(key)
}

/// The payload digest at `root`, from the cache when `id` is an immutable
/// source and it is there, else computed (and remembered when cacheable).
pub(super) fn payload_digest_cached(
    cache: Option<&Path>,
    id: &PackageId,
    root: &Path,
) -> Result<Digest, Ineligible> {
    let Some(cache) = cache.filter(|_| is_immutable(id, root)) else {
        return payload_digest(root);
    };
    let path = entry_path(cache, id);
    if let Some(digest) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<Digest>().ok())
    {
        return Ok(digest);
    }
    let digest = payload_digest(root)?;
    let _ = write(&path, &digest);
    Ok(digest)
}

fn write(path: &Path, digest: &Digest) -> std::io::Result<()> {
    let dir = path.parent().expect("an entry path has a parent");
    std::fs::create_dir_all(dir)?;
    // Same-directory temporary, then rename, so a reader never sees a torn file.
    let tmp = dir.join(format!(".tmp-{}", std::process::id()));
    std::fs::write(&tmp, digest.to_string())?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_crate(tmp: &Path, source: &str) -> (PackageId, PathBuf) {
        let root = tmp.join(format!(".cargo/registry/src/{source}/widget-lib-1.0.0"));
        std::fs::create_dir_all(root.join("skills/s")).unwrap();
        std::fs::write(root.join("skills/s/SKILL.md"), "one").unwrap();
        (PackageId::new(CARGO_PM, "widget-lib", "1.0.0"), root)
    }

    fn entries(cache: &Path) -> usize {
        std::fs::read_dir(cache.join(CACHE_DIR))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    #[test]
    fn a_registry_crate_is_hashed_once_and_then_read_from_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let (id, root) = registry_crate(tmp.path(), "index.crates.io-1949cf8c6b5b557f");

        let first = payload_digest_cached(Some(&cache), &id, &root).unwrap();
        assert_eq!(entries(&cache), 1, "miss: computed and written");

        // The same package now answers from the cache even if the tree
        // differs, which proves a hit reads the entry rather than hashing.
        std::fs::write(root.join("skills/s/SKILL.md"), "two").unwrap();
        assert_eq!(
            payload_digest_cached(Some(&cache), &id, &root).unwrap(),
            first
        );
        assert_ne!(payload_digest(&root).unwrap(), first);

        // Another version is another key.
        let other = PackageId::new(CARGO_PM, "widget-lib", "1.0.1");
        assert!(!is_immutable(&other, &root), "the directory names 1.0.0");
    }

    #[test]
    fn the_key_names_both_the_package_and_its_version() {
        let cache = Path::new("/cache");
        let id = |name: &str, version: &str| PackageId::new(CARGO_PM, name, version);
        let base = entry_path(cache, &id("widget-lib", "1.0.0"));
        assert_eq!(base, entry_path(cache, &id("widget-lib", "1.0.0")));
        assert_ne!(base, entry_path(cache, &id("widget-lib", "1.0.1")));
        assert_ne!(base, entry_path(cache, &id("other", "1.0.0")));
    }

    #[test]
    fn another_version_with_other_content_is_digested_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let source = "index.crates.io-1949cf8c6b5b557f";
        let (id, root) = registry_crate(tmp.path(), source);
        let first = payload_digest_cached(Some(&cache), &id, &root).unwrap();
        assert_eq!(entries(&cache), 1);

        let next_root = tmp
            .path()
            .join(format!(".cargo/registry/src/{source}/widget-lib-1.0.1"));
        std::fs::create_dir_all(next_root.join("skills/s")).unwrap();
        std::fs::write(next_root.join("skills/s/SKILL.md"), "different").unwrap();
        let next = PackageId::new(CARGO_PM, "widget-lib", "1.0.1");
        assert!(is_immutable(&next, &next_root));

        let digest = payload_digest_cached(Some(&cache), &next, &next_root).unwrap();
        assert_eq!(digest, payload_digest(&next_root).unwrap());
        assert_ne!(digest, first, "not the other version's cached digest");
        assert_eq!(entries(&cache), 2, "its own entry");
    }

    #[test]
    fn path_and_git_sources_are_never_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let (id, root) = registry_crate(tmp.path(), "index.crates.io-1949cf8c6b5b557f");

        // The same layout outside a registry source dir: a path dependency.
        let path_root = tmp.path().join("widget-lib-1.0.0");
        std::fs::create_dir_all(path_root.join("skills/s")).unwrap();
        std::fs::write(path_root.join("skills/s/SKILL.md"), "one").unwrap();
        let first = payload_digest_cached(Some(&cache), &id, &path_root).unwrap();
        assert_eq!(entries(&cache), 0);
        std::fs::write(path_root.join("skills/s/SKILL.md"), "two").unwrap();
        assert_ne!(
            payload_digest_cached(Some(&cache), &id, &path_root).unwrap(),
            first,
            "an edit to a path dependency is seen"
        );

        let git = tmp.path().join(".cargo/git/checkouts/widget-abc/1234567");
        std::fs::create_dir_all(&git).unwrap();
        payload_digest_cached(Some(&cache), &id, &git).unwrap();
        assert_eq!(entries(&cache), 0);

        // An unrelated registry source is not crates.io.
        let (_, other) = registry_crate(tmp.path(), "my-registry-0123");
        payload_digest_cached(Some(&cache), &id, &other).unwrap();
        assert_eq!(entries(&cache), 0);

        // No cache dir: always hashes.
        payload_digest_cached(None, &id, &root).unwrap();
        assert_eq!(entries(&cache), 0);
    }

    #[test]
    fn the_digest_scheme_version_is_part_of_the_key() {
        let id = PackageId::new(CARGO_PM, "widget-lib", "1.0.0");
        let cache = Path::new("/cache");
        let current = entry_path(cache, &id);
        // Same inputs under another domain prefix land elsewhere.
        let mut hasher = Sha256::new();
        hasher.update(b"symposium-payload-v2\0");
        hasher.update(id.name.as_bytes());
        hasher.update([0]);
        hasher.update(id.version.as_bytes());
        let v2: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_ne!(current, cache.join(CACHE_DIR).join(v2));
    }

    #[test]
    fn a_corrupt_entry_or_unwritable_cache_falls_back_to_hashing() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let (id, root) = registry_crate(tmp.path(), "index.crates.io-1949cf8c6b5b557f");
        let expected = payload_digest(&root).unwrap();

        let path = entry_path(&cache, &id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(
            payload_digest_cached(Some(&cache), &id, &root).unwrap(),
            expected
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            expected.to_string(),
            "the corrupt entry is replaced"
        );

        // A cache "directory" that is a file cannot be written to.
        let blocked = tmp.path().join("blocked");
        std::fs::write(&blocked, "x").unwrap();
        assert_eq!(
            payload_digest_cached(Some(&blocked), &id, &root).unwrap(),
            expected
        );
    }
}
