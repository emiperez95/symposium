//! `cargo agents plugin digest <crate|dir>`: print the payload digest an audits
//! file matches a crate on.
//!
//! Exists so that tooling outside Symposium (the runner that maintains an audits
//! list) hashes exactly as the client does, instead of reimplementing
//! [`payload_digest`](super::digest::payload_digest).

use std::path::Path;

use anyhow::{Result, bail};

use super::digest::payload_digest;
use crate::config::Symposium;
use crate::report::{DigestOutcome, ReportEvent};

/// Compute and report the digest for `target`: an existing directory is a crate
/// root; anything else is a crate name, resolved the way `crate-info` resolves
/// it (relative directories are taken from `cwd`).
///
/// Fails, after reporting the reason, when the payload is not eligible for a
/// digest or the crate cannot be resolved.
pub async fn run(sym: &Symposium, cwd: &Path, target: &str) -> Result<()> {
    let dir = cwd.join(target);
    let (root, krate) = if dir.is_dir() {
        (dir, None)
    } else {
        let found = crate::crate_command::resolve_crate(sym, target, None, cwd).await?;
        (
            found.root,
            Some(format!("{} {}", found.name, found.version)),
        )
    };
    match payload_digest(&root) {
        Ok(digest) => {
            tracing::info!(
                report = %ReportEvent::PayloadDigest {
                    target: target.to_string(),
                    krate,
                    outcome: DigestOutcome::Digest(digest),
                },
            );
            Ok(())
        }
        Err(why) => {
            tracing::info!(
                report = %ReportEvent::PayloadDigest {
                    target: target.to_string(),
                    krate,
                    outcome: DigestOutcome::Ineligible(why.to_string()),
                },
            );
            bail!("the payload of `{target}` is not eligible for a digest")
        }
    }
}
