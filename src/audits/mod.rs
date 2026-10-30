//! Audits: what the trusted registries say about a crate's plugin payload.
//!
//! A registry (a git cache, a `path` registry, `~/.symposium/plugins/`) may
//! carry `audits/<crate>.toml`, one file per crate, in the format
//! [`symposium_sdk::audits`] defines. This module reads them. It is the only
//! reader: the package managers stay ignorant of audits and only say where a
//! registry's content lives ([`PackageManager::content_dir`]).
//!
//! Two facts come out of a file, and they are merged across registries:
//!
//! - **Audited digests** — "this payload was reviewed". An audit from *any*
//!   registry counts, and the lookup remembers which registry it came from
//!   ([`AuditIndex::audited_by`]), so a later "audited by registry X" can say
//!   so. Nothing in this module makes an audited plugin load without a
//!   prompt: that decision is enablement's.
//! - **Revocations** — "this release or payload must not load". One from *any*
//!   registry applies ([`AuditIndex::revocation`]). A revocation names a
//!   semver requirement or a payload [`Digest`]; it applies to cargo-sourced
//!   plugins only, since a registry or the workspace is a trust root and a
//!   revocation has nothing to say about what the user put there.
//!
//! Looking up a crate reads only `audits/<normalized-crate>.toml` in each
//! registry, so a crate nobody audited costs one failed `open` per registry.
//! The payload digest is computed ([`digest`]) only when a file needs it, and remembered for immutable (crates.io) sources.
//!
//! **The client is lenient** where the maintaining tooling is strict: a
//! malformed entry (bad digest string, an entry that names both or neither of
//! `versions` and `digest`, an unparsable requirement) is dropped with a
//! warning, and a file with an unknown `format` is ignored whole. Warnings go
//! to `tracing`, never to stdout, because this runs on the hook path where
//! stdout belongs to the agent.
//!
//! [`PackageManager::content_dir`]: crate::pm::PackageManager::content_dir

use std::path::{Path, PathBuf};

use symposium_sdk::audits::{AUDITS_DIR, Digest, FORMAT, RawAudit, RawRevoked, RevokeReason};
use symposium_sdk::pm::{CARGO_PM, PackageId};

mod cache;
pub mod digest;
pub mod digest_command;

/// A file larger than this is ignored: audits are small, and the file is read
/// whole.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One trusted registry's content directory, named for the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSource {
    /// The configured registry name (what its plugins are attributed to).
    pub registry: String,
    /// The registry's root directory; audits live in `<dir>/audits/`.
    pub dir: PathBuf,
}

/// The audits of every trusted registry, read on demand.
#[derive(Debug, Clone, Default)]
pub struct AuditIndex {
    sources: Vec<AuditSource>,
    /// Where payload digests of immutable sources are remembered, if anywhere.
    cache_dir: Option<PathBuf>,
}

/// What the registries' audits say about one crate's payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuditLookup {
    /// A registry audited exactly this payload.
    Audited(String),
    /// A registry audited this crate, but not this payload: the audit does not
    /// cover this release's content.
    PayloadDiffers,
    /// A registry has `[[audit]]` entries for this crate, but this plugin's
    /// payload cannot be checked against them: it is not eligible for a digest
    /// (it fetches skills from outside the crate, or is too large).
    Ineligible,
    /// No registry audited this crate (or the package is not cargo-sourced).
    #[default]
    Unaudited,
}

/// A revocation that matched, and who issued it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revocation {
    /// The registry whose audits file revoked the package.
    pub registry: String,
    pub reason: RevokeReason,
}

/// The sentence status output and the activation report both use: who issued
/// the revocation and why.
impl std::fmt::Display for Revocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "revoked by registry `{}`: it {}",
            self.registry,
            self.reason.sentence()
        )
    }
}

impl AuditIndex {
    pub fn new(sources: Vec<AuditSource>) -> Self {
        Self {
            sources,
            cache_dir: None,
        }
    }

    /// Remember the digests of immutable (crates.io) sources under
    /// `<cache_dir>/audit-digests/`. Path, workspace and git sources are never
    /// cached, and a cache failure falls back to hashing.
    pub fn with_cache_dir(mut self, cache_dir: &Path) -> Self {
        self.cache_dir = Some(cache_dir.to_path_buf());
        self
    }

    fn digest_of(&self, id: &PackageId, root: &Path) -> Result<Digest, digest::Ineligible> {
        cache::payload_digest_cached(self.cache_dir.as_deref(), id, root)
    }

    /// Everything the registries say about `crate_name`: reads
    /// `audits/<normalized-name>.toml` from each.
    pub fn for_crate(&self, crate_name: &str) -> CrateAudits {
        let mut out = CrateAudits::default();
        let Some(file_name) = audit_file_name(crate_name) else {
            return out;
        };
        for source in &self.sources {
            let path = source.dir.join(AUDITS_DIR).join(&file_name);
            let Some(text) = read_small(&path) else {
                continue;
            };
            let (parsed, warnings) = parse_file(&source.registry, crate_name, &text);
            for warning in warnings {
                tracing::warn!(registry = %source.registry, path = %path.display(), "{warning}");
            }
            if let Some(parsed) = parsed {
                out.merge(parsed);
            }
        }
        out
    }

    /// Which registry audited `digest` for `crate_name`, if any. This is the
    /// lookup enablement uses to say "audited by registry X"; the first
    /// registry (in configuration order) that lists the digest wins.
    pub fn audited_by(&self, crate_name: &str, digest: &Digest) -> Option<String> {
        self.for_crate(crate_name)
            .audited_by(digest)
            .map(str::to_string)
    }

    /// Whether any trusted registry has an `audits/` directory at all. Cheap:
    /// the hook path uses it to decide whether audited loading can matter.
    pub fn any_audits_dir(&self) -> bool {
        self.sources.iter().any(|s| s.dir.join(AUDITS_DIR).is_dir())
    }

    /// Whether the registries audited the payload at `root`, the content of
    /// the cargo-sourced package `id`. The payload is hashed only when the
    /// crate has an audits file with an `[[audit]]` entry. A package of
    /// another package manager, or a crate with no `[[audit]]` entry, is
    /// `Unaudited`; a crate with entries whose payload cannot be digested is
    /// `Ineligible`.
    pub fn lookup(&self, id: &PackageId, root: &Path) -> AuditLookup {
        if id.pm != CARGO_PM {
            return AuditLookup::Unaudited;
        }
        self.lookup_in(&self.for_crate(&id.name), id, root)
    }

    /// [`lookup`](Self::lookup) against `audits`, which must be
    /// `self.for_crate(&id.name)`: for a caller that already read the crate's
    /// files (to decide whether a lookup is worth a fetch) and should not
    /// read and parse them again.
    pub fn lookup_in(&self, audits: &CrateAudits, id: &PackageId, root: &Path) -> AuditLookup {
        if id.pm != CARGO_PM || audits.audited.is_empty() {
            return AuditLookup::Unaudited;
        }
        match self.digest_of(id, root) {
            Ok(d) => match audits.audited_by(&d) {
                Some(registry) => AuditLookup::Audited(registry.to_string()),
                None => AuditLookup::PayloadDiffers,
            },
            Err(why) => {
                tracing::debug!(id = %id, %why, "no payload digest; an audit cannot match");
                AuditLookup::Ineligible
            }
        }
    }

    /// Whether a registry revoked `id`, a cargo-sourced package whose content
    /// is at `root`. `None` for any other package manager's package.
    ///
    /// Matches by version (the resolved version in `id`, skipped when it is not
    /// valid semver) or by payload digest. The digest is computed only when a
    /// file holds a digest revocation, and a payload that is
    /// [ineligible](digest::Ineligible) for a digest simply cannot match one.
    pub fn revocation(&self, id: &PackageId, root: &Path) -> Option<Revocation> {
        if id.pm != CARGO_PM {
            return None;
        }
        let audits = self.for_crate(&id.name);
        if audits.revoked.is_empty() {
            return None;
        }
        let version = semver::Version::parse(&id.version).ok();
        let digest = audits
            .has_digest_revocation()
            .then(|| self.digest_of(id, root))
            .and_then(|result| match result {
                Ok(d) => Some(d),
                Err(why) => {
                    tracing::debug!(id = %id, %why, "no payload digest; digest revocations cannot match");
                    None
                }
            });
        audits.revocation(version.as_ref(), digest.as_ref())
    }
}

/// What the registries said about one crate, merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrateAudits {
    /// `(registry, digest)` for every audited payload.
    audited: Vec<(String, Digest)>,
    revoked: Vec<RevokedEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RevokedEntry {
    registry: String,
    selector: Selector,
    reason: RevokeReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selector {
    Versions(semver::VersionReq),
    Digest(Digest),
}

impl CrateAudits {
    fn merge(&mut self, other: CrateAudits) {
        self.audited.extend(other.audited);
        self.revoked.extend(other.revoked);
    }

    /// Does any registry audit (not merely revoke) the crate?
    pub fn has_audits(&self) -> bool {
        !self.audited.is_empty()
    }

    /// No audits and no revocations.
    pub fn is_empty(&self) -> bool {
        self.audited.is_empty() && self.revoked.is_empty()
    }

    /// The registry that audited `digest`, if any.
    pub fn audited_by(&self, digest: &Digest) -> Option<&str> {
        self.audited
            .iter()
            .find(|(_, d)| d == digest)
            .map(|(registry, _)| registry.as_str())
    }

    /// Does any revocation name a payload digest (so the payload must be
    /// hashed to evaluate them)?
    pub fn has_digest_revocation(&self) -> bool {
        self.revoked
            .iter()
            .any(|r| matches!(r.selector, Selector::Digest(_)))
    }

    /// The first revocation matching this version or digest. A requirement
    /// follows semver's rule that a pre-release only matches a requirement
    /// that itself names a pre-release.
    pub fn revocation(
        &self,
        version: Option<&semver::Version>,
        digest: Option<&Digest>,
    ) -> Option<Revocation> {
        self.revoked
            .iter()
            .find(|r| match &r.selector {
                Selector::Versions(req) => version.is_some_and(|v| req.matches(v)),
                Selector::Digest(d) => digest == Some(d),
            })
            .map(|r| Revocation {
                registry: r.registry.clone(),
                reason: r.reason,
            })
    }
}

/// A crate name in its comparison form: lowercase, `-` as `_` (what cargo
/// treats as the same name, and what an audits file name uses).
fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "_")
}

/// The file name for `crate_name`'s audits: lowercase, `-` as `_`, `.toml`.
/// `None` for a name that is not a plain crate name, so a name can never reach
/// outside the `audits/` directory.
pub fn audit_file_name(crate_name: &str) -> Option<String> {
    let normalized = normalize(crate_name);
    let plain = !normalized.is_empty()
        && normalized
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    plain.then(|| format!("{normalized}.toml"))
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_FILE_BYTES {
        tracing::warn!(path = %path.display(), "ignoring oversized audits file");
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "cannot read audits file");
            None
        }
    }
}

/// Parse one audits file leniently, returning what it yields (`None` ignores
/// the whole file) and a warning per thing dropped. Warnings are returned so
/// the caller decides where they go, and so tests can read them.
fn parse_file(registry: &str, crate_name: &str, text: &str) -> (Option<CrateAudits>, Vec<String>) {
    let mut warnings = Vec::new();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("ignoring audits file that is not valid TOML: {e}"));
            return (None, warnings);
        }
    };
    match table.get("format").and_then(toml::Value::as_integer) {
        Some(f) if f == i64::from(FORMAT) => {}
        other => {
            warnings.push(format!(
                "ignoring audits file with unsupported format {} (this symposium reads format {FORMAT})",
                other.map_or("(missing)".to_string(), |f| f.to_string()),
            ));
            return (None, warnings);
        }
    }
    match table.get("crate").and_then(toml::Value::as_str) {
        Some(name) if normalize(name) == normalize(crate_name) => {}
        _ => {
            warnings.push(format!(
                "ignoring audits file whose `crate` is not `{crate_name}`"
            ));
            return (None, warnings);
        }
    }

    let mut out = CrateAudits::default();
    for (i, value) in array(&table, "audit", &mut warnings)
        .into_iter()
        .enumerate()
    {
        let digest = value
            .clone()
            .try_into::<RawAudit>()
            .ok()
            .and_then(|raw| raw.digest.parse::<Digest>().ok());
        match digest {
            Some(digest) => out.audited.push((registry.to_string(), digest)),
            None => warnings.push(format!("dropping malformed [[audit]] entry #{}", i + 1)),
        }
    }
    for (i, value) in array(&table, "revoked", &mut warnings)
        .into_iter()
        .enumerate()
    {
        match value
            .clone()
            .try_into::<RawRevoked>()
            .ok()
            .and_then(revoked_entry)
        {
            Some((selector, reason)) => out.revoked.push(RevokedEntry {
                registry: registry.to_string(),
                selector,
                reason,
            }),
            None => warnings.push(format!("dropping malformed [[revoked]] entry #{}", i + 1)),
        }
    }
    (Some(out), warnings)
}

/// The entries of the `[[key]]` array of tables, empty when the key is absent.
/// A key that is present but not an array (`[revoked]` for `[[revoked]]`, or
/// `revoked = "x"`) yields nothing too, but says so: dropping it silently would
/// lose every revocation in the file because of one missing bracket.
fn array<'a>(
    table: &'a toml::Table,
    key: &str,
    warnings: &mut Vec<String>,
) -> Vec<&'a toml::Value> {
    match table.get(key) {
        None => Vec::new(),
        Some(toml::Value::Array(a)) => a.iter().collect(),
        Some(other) => {
            warnings.push(format!(
                "ignoring `{key}`: expected an array of tables ([[{key}]]), found {}",
                other.type_str()
            ));
            Vec::new()
        }
    }
}

/// A revocation's selector and reason, or `None` when it is malformed.
fn revoked_entry(raw: RawRevoked) -> Option<(Selector, RevokeReason)> {
    let selector = match (raw.versions, raw.digest) {
        (Some(req), None) => Selector::Versions(semver::VersionReq::parse(&req).ok()?),
        (None, Some(digest)) => Selector::Digest(digest.parse().ok()?),
        _ => return None,
    };
    Some((selector, raw.reason))
}

#[cfg(test)]
mod tests;
