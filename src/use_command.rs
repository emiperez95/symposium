//! `cargo agents use` — explicit plugin enablement.
//!
//! Enablement is the consent axis: the workspace and the configured
//! registries are trust roots, but a dependency is not, so a plugin embedded
//! in a dependency runs only once the user says so. `use` is the durable,
//! by-name form of that decision — it writes a [`UseEntry`] into `[plugins]
//! use`, scoped to the current workspace by default or to every workspace
//! with `--global`.
//!
//! It is also what wakes a *dormant* registry plugin (one whose manifest
//! names no dependency, so nothing else would ever gate it on —
//! [`PluginManifest::requires_use`](crate::plugins::PluginManifest::requires_use)).
//!
//! `use` only adds to what *may* run; activation predicates still decide
//! when it applies. `--remove` is the inverse, and re-syncs so the plugin's
//! skills are reaped straight away.

use std::path::Path;

use anyhow::{Context, Result, bail};
use symposium_install::UpdateLevel;

use crate::config::{Symposium, UseEntry};
use crate::pm::{PackageId, PmRegistry};
use crate::report::ReportEvent;

/// Record an enablement for `name` and sync so its skills install now.
pub async fn use_plugin(
    sym: &mut Symposium,
    cwd: &Path,
    name: &str,
    global: bool,
    update: UpdateLevel,
) -> Result<()> {
    let deps = sym.workspace_deps(cwd);
    let pms = sym.package_managers(&deps);
    let registry = crate::plugins::load_registry(sym).await;
    let id = pms.identify(&registry.plugins, name);

    // `disable` outranks every way of enabling a plugin, `use` included, so
    // recording an entry here would report success and then install nothing.
    // Re-enabling is dropping the `disable` entry, which is a deliberate edit
    // rather than something `use` undoes on the user's behalf.
    if sym
        .config
        .plugins
        .is_disabled(&pms, &PackageId::any_version(&id.pm, name))
    {
        bail!("`{name}` is disabled in `[plugins] disable`; remove that entry to enable it again");
    }

    // A configured registry is a trust root: what it offers is already
    // enabled by configuration, so there is nothing to record. The exception
    // is a dormant plugin, for which `use` is exactly the wake-up call.
    let registry_plugin = registry
        .plugins
        .iter()
        .find(|p| pms.identity(&p.canonical) == id);
    let dormant = registry_plugin.is_some_and(|p| p.manifest.requires_use);
    let already_trusted = registry_plugin.is_some() && !dormant;
    if already_trusted {
        tracing::info!(
            report = %ReportEvent::Info {
                message: format!(
                    "`{name}` is already available from a configured registry; nothing to enable"
                ),
            },
        );
        return Ok(());
    }

    let workspace_root = deps.load().map(|ws| ws.root.clone());
    if !global && let Err(e) = deps.require() {
        bail!("{e}; pass --global to enable `{name}` everywhere");
    }

    if !dormant {
        resolve_name(&pms, &id, name).await?;
    }

    let entry = match &workspace_root {
        _ if global => UseEntry::Global(name.to_string()),
        Some(root) => UseEntry::Workspace {
            name: name.to_string(),
            workspace: root.clone(),
        },
        None => unreachable!("checked above"),
    };

    let already_enabled = sym.config.plugins.used.iter().any(|e| {
        in_scope(e, global, workspace_root.as_deref())
            && pms.identify(&registry.plugins, e.name()) == id
    });
    if already_enabled {
        tracing::info!(
            report = %ReportEvent::Info {
                message: format!("`{name}` is already enabled; nothing changed"),
            },
        );
    } else {
        sym.config.plugins.used.push(entry);
        sym.save_config().context("failed to write user config")?;
        tracing::info!(
            report = %ReportEvent::PluginEnabled {
                name: name.to_string(),
                global,
            },
        );
    }

    // Install now rather than waiting for the next sync.
    if workspace_root.is_some() {
        crate::sync::sync(sym, &deps, update).await?;
    }
    Ok(())
}

/// Drop a previously recorded enablement and re-sync so the plugin's skills
/// are reaped now.
///
/// The scope must match: without `--global` this removes the entry recorded
/// for the current workspace, with it the unscoped entry. A scope mismatch is
/// an error rather than a silent success.
pub async fn remove_plugin(
    sym: &mut Symposium,
    cwd: &Path,
    name: &str,
    global: bool,
    update: UpdateLevel,
) -> Result<()> {
    let deps = sym.workspace_deps(cwd);
    let workspace_root = deps.load().map(|ws| ws.root.clone());
    let pms = sym.package_managers(&deps);
    let registry = crate::plugins::load_registry(sym).await;
    let id = pms.identify(&registry.plugins, name);

    let used = &mut sym.config.plugins.used;
    let before = used.len();
    used.retain(|entry| {
        !in_scope(entry, global, workspace_root.as_deref())
            || pms.identify(&registry.plugins, entry.name()) != id
    });
    if used.len() == before {
        let scope = if global { "--global" } else { "this workspace" };
        bail!("no `use` entry for `{name}` ({scope}); see `cargo agents status`");
    }
    sym.save_config().context("failed to write user config")?;
    tracing::info!(
        report = %ReportEvent::PluginRemoved {
            name: name.to_string(),
            global,
        },
    );

    if workspace_root.is_some() {
        crate::sync::sync(sym, &deps, update).await?;
    }
    Ok(())
}

fn in_scope(entry: &UseEntry, global: bool, workspace_root: Option<&Path>) -> bool {
    match entry {
        UseEntry::Global(_) => global,
        UseEntry::Workspace { .. } => {
            !global && workspace_root.is_some_and(|root| entry.applies_in(root))
        }
    }
}

/// Check that `name` resolves to something before recording it: a workspace
/// dependency (offline-friendly) or a registry search hit.
async fn resolve_name(pms: &PmRegistry, id: &PackageId, name: &str) -> Result<()> {
    let is_workspace_dep = pms
        .list_deps()
        .await
        .unwrap_or_default()
        .iter()
        .any(|dep| pms.identity(dep) == *id);
    if is_workspace_dep {
        return Ok(());
    }

    let found = pms.search(name).await.iter().any(|(_, info)| {
        pms.identity(&info.id) == pms.identity(&PackageId::any_version(&info.id.pm, name))
    });
    if found {
        return Ok(());
    }
    bail!("no crate or plugin named `{name}` found (try `cargo agents search {name}`)")
}
