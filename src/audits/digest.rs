//! The payload digest: a hash of exactly the part of a crate that an audit
//! covers.
//!
//! What an audit reviews is the *plugin payload*: the manifest that says what
//! the crate contributes, and the skill content that manifest names. A crate's
//! code is out of scope, so a release that changes only `src/` keeps the same
//! digest, and one audit covers every such release. The digest is the match
//! key an audits file uses, which is why it must come out identical on every
//! platform and in tooling outside Symposium (`cargo agents plugin digest`
//! exists so that tooling can ask Symposium instead of reimplementing this).
//!
//! # What is hashed
//!
//! A set of named entries, framed and sorted bytewise by name:
//!
//! 1. `Symposium.toml` at the crate root, as the raw bytes, when present.
//! 2. `[package.metadata.symposium]` from `Cargo.toml`, when present, as
//!    canonical compact JSON (keys sorted, no whitespace, TOML datetimes as
//!    strings) under the name `Cargo.toml#package.metadata.symposium`. This is
//!    the only canonicalized part; nothing else in `Cargo.toml` is read.
//! 3. Every file, of any type, in each *skill directory* the manifest names
//!    inside the crate: the `[[skills]] source.path` groups, including the
//!    default `skills/` group unless `[defaults] skills = false`. A skill
//!    directory is one that holds a `SKILL.md`, and its whole tree counts. A
//!    file reachable twice is hashed once. Names are paths relative to the
//!    crate root with `/` separators.
//!
//! The input is the domain-separation prefix `symposium-payload-v1\0`, then
//! per entry `u64` big-endian name length, the name, `u64` big-endian content
//! length, the content. Bytes are hashed as published: no line-ending
//! normalization.
//!
//! Group predicates are not consulted: every group counts, so the digest does
//! not depend on the workspace it is computed in.
//!
//! # When there is no digest
//!
//! A payload that cannot be pinned gets [`Ineligible`], not a digest, and the
//! caller falls back to asking: a `source.git` skill group (what it fetches
//! can change under the same crate release), a skill path that leaves the
//! crate, a broken symlink or a directory loop in a skill directory, an entry
//! that is neither file nor directory, or more than [`MAX_PAYLOAD_BYTES`] of
//! content. Any I/O error other than a plain "not there" (permissions, a
//! dangling link where a manifest or group directory should be) is likewise
//! ineligible: it is never read as absence.
//!
//! Directory traversal follows symlinks and refuses loops through
//! [`crate::dir_walk`], like every other source-tree walk.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use symposium_sdk::audits::Digest;

use crate::dir_walk::{Ancestors, EntryKind, resolved_kind};
use crate::plugins::PluginSource;

/// Most content bytes a payload may have and still get a digest (4 MiB).
pub const MAX_PAYLOAD_BYTES: u64 = 4 * 1024 * 1024;

/// Prefix of the hashed input, separating this digest from any other use of
/// SHA-256. The `v1` is the version of this scheme.
pub(super) const DOMAIN: &[u8] = b"symposium-payload-v1\0";

/// Entry name of the crate-root plugin manifest.
const MANIFEST_ENTRY: &str = "Symposium.toml";

/// Entry name of the canonicalized `[package.metadata.symposium]` table.
const METADATA_ENTRY: &str = "Cargo.toml#package.metadata.symposium";

/// Why a payload has no digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ineligible {
    /// The crate root is not a directory.
    NotADirectory(PathBuf),
    /// A manifest source could not be read or does not validate as a plugin
    /// manifest, so there is no plugin to pin.
    Manifest(String),
    /// A `source.git` skill group: it fetches content the crate release does
    /// not pin.
    GitSkillSource(String),
    /// A skill path (as written in the manifest) resolves outside the crate.
    EscapesCrate(String),
    /// A symlink in a skill directory that points nowhere.
    BrokenLink(String),
    /// A directory symlink in a skill directory that leads back into the
    /// directory it sits in.
    LinkLoop(String),
    /// An entry in a skill directory that is neither a file nor a directory,
    /// or whose path is not valid UTF-8.
    Unsupported(String),
    /// The payload is larger than [`MAX_PAYLOAD_BYTES`].
    TooLarge,
    /// A file could not be read.
    Unreadable(String),
}

impl fmt::Display for Ineligible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(p) => write!(f, "{} is not a directory", p.display()),
            Self::Manifest(why) => write!(f, "the plugin manifest is not usable: {why}"),
            Self::GitSkillSource(url) => write!(
                f,
                "a skill group fetches `source.git = \"{url}\"`, which a crate release does not pin"
            ),
            Self::EscapesCrate(path) => {
                write!(f, "skill path `{path}` leaves the crate directory")
            }
            Self::BrokenLink(path) => write!(f, "`{path}` is a broken symlink"),
            Self::LinkLoop(path) => write!(f, "`{path}` is a symlink loop"),
            Self::Unsupported(path) => write!(
                f,
                "`{path}` is not a regular file or directory, or its path is not UTF-8"
            ),
            Self::TooLarge => write!(f, "the payload is larger than {} bytes", MAX_PAYLOAD_BYTES),
            Self::Unreadable(why) => write!(f, "cannot read the payload: {why}"),
        }
    }
}

impl std::error::Error for Ineligible {}

/// The payload digest of the crate whose source is at `root`.
pub fn payload_digest(root: &Path) -> Result<Digest, Ineligible> {
    let entries = payload_entries(root)?;
    Ok(hash_entries(&entries))
}

/// The framed entries of the payload, by name.
fn payload_entries(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, Ineligible> {
    if !root.is_dir() {
        return Err(Ineligible::NotADirectory(root.to_path_buf()));
    }
    let mut payload = Payload::default();

    // 1. The manifest file, raw.
    let manifest_path = root.join(crate::pm::layout::MANIFEST_FILE);
    let manifest_text = if stat_file(&manifest_path, MANIFEST_ENTRY)?.is_some() {
        let bytes = payload.read(&manifest_path)?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| Ineligible::Manifest("Symposium.toml is not UTF-8".into()))?;
        payload.put(MANIFEST_ENTRY.to_string(), bytes);
        Some(text)
    } else {
        None
    };

    // 2. The metadata table, canonicalized.
    let cargo_toml = root.join("Cargo.toml");
    let metadata = if stat_file(&cargo_toml, "Cargo.toml")?.is_some() {
        crate::crate_metadata::symposium_metadata(&cargo_toml)
            .map_err(|e| Ineligible::Manifest(format!("{e:#}")))?
    } else {
        None
    };
    if let Some(table) = &metadata {
        let mut json = String::new();
        canonical_json(&toml::Value::Table(table.clone()), &mut json);
        payload.charge(json.len() as u64)?;
        payload.put(METADATA_ENTRY.to_string(), json.into_bytes());
    }

    // 3. The skill directories the (merged, validated) manifest names.
    let manifest = crate::plugins::package_manifest(metadata, manifest_text.as_deref(), "payload")
        .map_err(|e| Ineligible::Manifest(format!("{e:#}")))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|e| Ineligible::Unreadable(format!("{}: {e}", root.display())))?;
    for group in &manifest.skills {
        let rel = match &group.source {
            PluginSource::Git(url) => return Err(Ineligible::GitSkillSource(url.clone())),
            PluginSource::Path(rel) => rel,
        };
        // A group directory that does not exist contributes nothing; one that
        // exists but cannot be resolved (a dangling link, an I/O error) does
        // not get to pass as missing.
        let joined = root.join(rel);
        let group_dir = match joined.canonicalize() {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(&joined) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Ok(_) => return Err(Ineligible::BrokenLink(rel.display().to_string())),
                    Err(e) => {
                        return Err(Ineligible::Unreadable(format!("{}: {e}", joined.display())));
                    }
                }
            }
            Err(e) => return Err(Ineligible::Unreadable(format!("{}: {e}", joined.display()))),
        };
        if !group_dir.starts_with(&canonical_root) {
            return Err(Ineligible::EscapesCrate(rel.display().to_string()));
        }
        if !group_dir.is_dir() {
            continue;
        }
        let mut skill_dirs = Vec::new();
        let mut ancestors = Ancestors::rooted_at(&group_dir);
        find_skill_dirs(&canonical_root, &group_dir, &mut ancestors, &mut skill_dirs)?;
        for skill_dir in skill_dirs {
            payload.add_tree(&canonical_root, &skill_dir)?;
        }
    }
    Ok(payload.entries)
}

/// Stat a manifest file at the crate root: `None` when it is truly absent,
/// an error when it exists but is not a readable regular file or when the
/// answer is anything but "not there" (a dangling link is not absence).
fn stat_file(path: &Path, name: &str) -> Result<Option<std::fs::Metadata>, Ineligible> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(meta)),
        Ok(_) => Err(Ineligible::Unsupported(name.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Ok(_) => Err(Ineligible::BrokenLink(name.to_string())),
                Err(e) => Err(Ineligible::Unreadable(format!("{name}: {e}"))),
            }
        }
        Err(e) => Err(Ineligible::Unreadable(format!("{name}: {e}"))),
    }
}

/// The skill directories under `dir` (those holding a `SKILL.md`): the same
/// discovery as `skills::find_skill_files_recursive`, except that it reports
/// what that walk swallows. A directory that cannot be listed, a link that
/// points nowhere or back into the walk, would each hide skill content that
/// the digest must cover, so none of them may pass as "no skills here".
fn find_skill_dirs(
    root: &Path,
    dir: &Path,
    ancestors: &mut Ancestors,
    out: &mut Vec<PathBuf>,
) -> Result<(), Ineligible> {
    let unreadable = |e: std::io::Error| Ineligible::Unreadable(format!("{}: {e}", dir.display()));
    let shown = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    // An installed copy (carries symposium's marker) is never a source.
    if dir
        .join(crate::sync::MARKER_FILE)
        .try_exists()
        .map_err(unreadable)?
    {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(unreadable)?;
        match resolved_kind(&path, file_type) {
            Err(_) => return Err(Ineligible::BrokenLink(shown(&path))),
            Ok(Some(EntryKind::Dir)) => {
                if !ancestors.enter(&path) {
                    return Err(Ineligible::LinkLoop(shown(&path)));
                }
                let result = find_skill_dirs(root, &path, ancestors, out);
                ancestors.leave();
                result?;
            }
            Ok(Some(EntryKind::File)) if entry.file_name() == "SKILL.md" => {
                out.push(dir.to_path_buf());
            }
            Ok(_) => {}
        }
    }
    Ok(())
}

/// Entries gathered so far, with the running content size.
#[derive(Default)]
struct Payload {
    entries: BTreeMap<String, Vec<u8>>,
    bytes: u64,
}

impl Payload {
    /// Read a file, refusing it before reading if it would push the payload
    /// over the cap.
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, Ineligible> {
        let len = std::fs::metadata(path)
            .map_err(|e| Ineligible::Unreadable(format!("{}: {e}", path.display())))?
            .len();
        self.charge(len)?;
        std::fs::read(path).map_err(|e| Ineligible::Unreadable(format!("{}: {e}", path.display())))
    }

    fn charge(&mut self, len: u64) -> Result<(), Ineligible> {
        self.bytes = self.bytes.saturating_add(len);
        if self.bytes > MAX_PAYLOAD_BYTES {
            Err(Ineligible::TooLarge)
        } else {
            Ok(())
        }
    }

    /// Add an entry. A name present already (a file reached through two
    /// groups) keeps its first bytes.
    fn put(&mut self, name: String, bytes: Vec<u8>) {
        self.entries.entry(name).or_insert(bytes);
    }

    /// Add every file under `dir`, named relative to `root`. `dir` is inside
    /// `root` (canonical).
    fn add_tree(&mut self, root: &Path, dir: &Path) -> Result<(), Ineligible> {
        let rel = dir
            .strip_prefix(root)
            .map_err(|_| Ineligible::EscapesCrate(dir.display().to_string()))?;
        let mut name = String::new();
        for component in rel.components() {
            let part = component
                .as_os_str()
                .to_str()
                .ok_or_else(|| Ineligible::Unsupported(dir.display().to_string()))?;
            name.push_str(part);
            name.push('/');
        }
        let mut ancestors = Ancestors::rooted_at(dir);
        self.walk(dir, &name, &mut ancestors)
    }

    /// `prefix` is the entry-name prefix of `dir` (empty or ending in `/`).
    fn walk(
        &mut self,
        dir: &Path,
        prefix: &str,
        ancestors: &mut Ancestors,
    ) -> Result<(), Ineligible> {
        let read = std::fs::read_dir(dir)
            .map_err(|e| Ineligible::Unreadable(format!("{}: {e}", dir.display())))?;
        for entry in read {
            let entry =
                entry.map_err(|e| Ineligible::Unreadable(format!("{}: {e}", dir.display())))?;
            let path = entry.path();
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                return Err(Ineligible::Unsupported(path.display().to_string()));
            };
            let name = format!("{prefix}{file_name}");
            let file_type = entry
                .file_type()
                .map_err(|e| Ineligible::Unreadable(format!("{name}: {e}")))?;
            match resolved_kind(&path, file_type) {
                Err(_) => return Err(Ineligible::BrokenLink(name)),
                Ok(None) => return Err(Ineligible::Unsupported(name)),
                Ok(Some(EntryKind::File)) => {
                    // The synthesized metadata entry's name is reserved.
                    if name == METADATA_ENTRY {
                        return Err(Ineligible::Unsupported(name));
                    }
                    // Reached through another group already: same name, same
                    // file, so it is neither read nor counted twice.
                    if !self.entries.contains_key(&name) {
                        let bytes = self.read(&path)?;
                        self.put(name, bytes);
                    }
                }
                Ok(Some(EntryKind::Dir)) => {
                    if !ancestors.enter(&path) {
                        return Err(Ineligible::LinkLoop(name));
                    }
                    let result = self.walk(&path, &format!("{name}/"), ancestors);
                    ancestors.leave();
                    result?;
                }
            }
        }
        Ok(())
    }
}

/// Hash the framed entries (already sorted by name, bytewise).
fn hash_entries(entries: &BTreeMap<String, Vec<u8>>) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    for (name, content) in entries {
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((content.len() as u64).to_be_bytes());
        hasher.update(content);
    }
    Digest::from_bytes(hasher.finalize().into())
}

/// Compact JSON with object keys sorted bytewise. TOML has no `null`; a
/// datetime is its string form and a non-finite float is spelled as a string,
/// since JSON has no number for it.
fn canonical_json(value: &toml::Value, out: &mut String) {
    match value {
        toml::Value::String(s) => out.push_str(&json_string(s)),
        toml::Value::Integer(i) => out.push_str(&i.to_string()),
        toml::Value::Float(f) if f.is_finite() => out.push_str(
            &serde_json::Number::from_f64(*f).map_or_else(String::new, |n| n.to_string()),
        ),
        toml::Value::Float(f) => out.push_str(&json_string(&f.to_string())),
        toml::Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        toml::Value::Datetime(d) => out.push_str(&json_string(&d.to_string())),
        toml::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        toml::Value::Table(table) => {
            let sorted: BTreeMap<&String, &toml::Value> = table.iter().collect();
            out.push('{');
            for (i, (key, item)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                canonical_json(item, out);
            }
            out.push('}');
        }
    }
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

#[cfg(test)]
mod tests;
