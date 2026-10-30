//! Crate command: fetch crate sources by name and version.

use std::path::Path;

use crate::config::Symposium;
use crate::crate_sources::normalize_crate_name;
use crate::pm::CargoPm;

/// Result of dispatching a command.
pub enum DispatchResult {
    /// Successful output to display.
    Ok(String),
    /// Error message.
    Err(String),
}

pub async fn dispatch_crate(
    sym: &Symposium,
    name: &str,
    version: Option<&str>,
    cwd: &Path,
) -> DispatchResult {
    tracing::debug!(%name, ?version, "crate-info dispatched");
    match resolve_crate(sym, name, version, cwd).await {
        Ok(found) => crate_info(&found.name, &found.version, &found.root),
        Err(e) => DispatchResult::Err(format!("{e}")),
    }
}

/// A crate resolved to its source directory.
pub struct ResolvedCrate {
    pub name: String,
    pub version: String,
    pub root: std::path::PathBuf,
}

/// Resolve a crate by name (and optional version) to its source, the way
/// `crate-info` does: a workspace member's own directory when no version is
/// asked for, otherwise whatever the cargo package manager fetches.
pub async fn resolve_crate(
    sym: &Symposium,
    name: &str,
    version: Option<&str>,
    cwd: &Path,
) -> Result<ResolvedCrate, anyhow::Error> {
    let deps = sym.workspace_deps(cwd);
    if version.is_none() {
        let normalized = normalize_crate_name(name);
        if let Some(member) = deps
            .member_crates()
            .iter()
            .find(|m| normalize_crate_name(&m.name) == normalized)
            && let Some(dir) = &member.path
        {
            return Ok(ResolvedCrate {
                name: member.name.clone(),
                version: member.version.to_string(),
                root: dir.clone(),
            });
        }
    }
    let id = CargoPm::id_for(name, version);
    let result = sym
        .package_managers(&deps)
        .fetch(&id, symposium_install::UpdateLevel::Check)
        .await?;
    Ok(ResolvedCrate {
        name: result.id.name,
        version: result.id.version,
        root: result.root,
    })
}

fn crate_info(name: &str, version: &str, source: &Path) -> DispatchResult {
    let output = format!(
        "Crate: {name}\nVersion: {version}\nSource: {}\n",
        source.display()
    );
    tracing::trace!(%output, "crate-info output");
    DispatchResult::Ok(output)
}
