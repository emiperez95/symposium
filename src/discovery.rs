//! Dependency discovery: which plugins the workspace's dependencies bring
//! within reach, and which of them the user has consented to.
//!
//! Discovery is the read side of the enablement axis. It runs in two phases:
//!
//! 1. list the workspace's dependencies ([`pm::workspace_dep_ids`]);
//! 2. ask each *untrusted* instance — the cargo transport — for the plugins
//!    its dependencies embed ([`PackageManager::active_plugins`]).
//!
//! Each such offer is then classified against the `[plugins]` config:
//! already enabled, auto-enabled (or auto-accepted), declined, or a candidate still awaiting
//! consent. Discovery itself neither fetches nor writes.
//!
//! On top of that read side sits the consent write side:
//! [`prompt_for_consent`] asks about each candidate and [`apply_consent`]
//! records the answers. The prompt is inert unless its [`Output`] is
//! interactive, so hook dispatch — and anything else an agent triggers —
//! can never block on stdin; those contexts get [`pending_candidates`] as a
//! `SessionStart` hint instead.
//!
//! Enablement matters because a dependency is deliberately *not* a trust
//! root: depending on a crate means compiling its code, not letting its
//! author inject agent context. Registry instances are trust roots — a
//! registry exists to curate plugins — but their plugins are loaded and
//! gated directly ([`plugins::load_registry`], evaluated by
//! [`Plugin::applies`]), so they never reach discovery. What discovery
//! classifies is exactly the untrusted offers: the dependency-embedded
//! plugins an ecosystem transport surfaces, which run only with consent.
//!
//! [`plugins::load_registry`]: crate::plugins::load_registry
//! [`Plugin::applies`]: crate::plugins::Plugin::applies
//!
//! [`pm::workspace_dep_ids`]: crate::pm::workspace_dep_ids
//! [`PackageManager::active_plugins`]: crate::pm::PackageManager::active_plugins

use std::collections::HashSet;
use std::path::Path;

use crate::pm::WorkspaceDeps;
use anyhow::{Context, Result};
use std::sync::Arc;
use symposium_install::UpdateLevel;

use crate::audits::{AuditIndex, AuditLookup, Revocation};
use crate::config::{AutoAccept, Symposium};
use crate::output::Output;
use crate::plugins::Plugin;
use crate::pm::{CARGO_PM, CargoPm, PackageId, PmRegistry};
use crate::report::ReportEvent;

/// Why a discovered offer is (or is not) enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enablement {
    /// Enabled by a `[plugins] use` entry naming it.
    Used,
    /// Enabled ahead of time by an explicit `[plugins] auto-enable` entry.
    AutoEnabled,
    /// Accepted without asking because `[plugins] auto-accept = "all"`, with
    /// no `auto-enable` entry naming it.
    AutoAccepted,
    /// Declined: `[plugins] disable` names it.
    Declined,
    /// Accepted without asking because a trusted registry audited exactly this
    /// payload (`auto-accept = "audit"`). Standing like `auto-enable` for the
    /// purpose of loading, but never recorded in config: it holds only while
    /// the audit and the payload match.
    Audited { registry: String },
    /// A trusted registry's audits revoke this release or payload. Outranks
    /// `use`, `auto-enable` and `auto-accept`; only `disable` outranks it.
    Revoked(Revocation),
    /// Nobody has decided yet — this is what a consent prompt would ask about.
    Candidate,
}

impl Enablement {
    /// Does this decision let the plugin run?
    pub fn is_enabled(&self) -> bool {
        matches!(
            self,
            Self::Used | Self::AutoEnabled | Self::AutoAccepted | Self::Audited { .. }
        )
    }
}

/// One plugin offer whose recommended dependency the workspace has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPlugin {
    /// The offering package-manager instance: an ecosystem transport
    /// (`cargo`) for a dependency-embedded plugin, or a registry's name.
    pub registry: String,
    /// The offered package.
    pub id: PackageId,
    /// The dependency this offer is a plugin for.
    pub recommends: String,
    /// A short human summary of what the plugin contributes, for the consent
    /// prompt.
    pub description: Option<String>,
    /// How the `[plugins]` config decided this offer.
    pub enablement: Enablement,
    /// What the registries' audits say about this payload. `None` means no
    /// lookup was made: it is computed only for an offer that reached the audit
    /// step (not disabled, revoked or already enabled), which is every
    /// [`Enablement::Candidate`] and [`Enablement::Audited`] offer.
    pub audit: Option<AuditLookup>,
}

impl DiscoveredPlugin {
    /// The name the user would type to enable this plugin.
    pub fn name(&self) -> &str {
        &self.id.name
    }
}

/// Every dependency-matched offer, grouped by what the config decided.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Already enabled: named by a `use` entry, or offered by a registry.
    pub active: Vec<DiscoveredPlugin>,
    /// Enabled by an explicit `[plugins] auto-enable` entry.
    pub auto_enabled: Vec<DiscoveredPlugin>,
    /// Accepted without asking by `[plugins] auto-accept = "all"`.
    pub auto_accepted: Vec<DiscoveredPlugin>,
    /// Accepted without asking because a registry audited the payload
    /// (`auto-accept = "audit"`). Nothing is written to config for these.
    pub audited: Vec<DiscoveredPlugin>,
    /// Awaiting consent — newly discovered and not yet decided.
    pub candidates: Vec<DiscoveredPlugin>,
    /// Declined previously, recorded in `[plugins] disable`.
    pub declined: Vec<DiscoveredPlugin>,
    /// Revoked by a trusted registry's audits. Never loaded, whatever else
    /// says it may be.
    pub revoked: Vec<DiscoveredPlugin>,
}

impl Discovery {
    /// Every offer that may run, whatever enabled it.
    pub fn enabled(&self) -> impl Iterator<Item = &DiscoveredPlugin> {
        self.active
            .iter()
            .chain(&self.auto_enabled)
            .chain(&self.auto_accepted)
            .chain(&self.audited)
    }

    /// Every offer in every bucket: those that may run (see
    /// [`enabled`](Self::enabled)), then candidates, declined and revoked.
    pub fn all(&self) -> impl Iterator<Item = &DiscoveredPlugin> {
        self.enabled()
            .chain(&self.candidates)
            .chain(&self.declined)
            .chain(&self.revoked)
    }
}

/// Discover the plugins offered for this workspace's dependencies.
///
/// `workspace_root` scopes the `use` entries that count (an entry can be
/// recorded for one workspace only). `active_plugins` fetches each dependency
/// cache-only — for a workspace dependency, into the source `cargo metadata`
/// already extracted (no probe, no network) — so every dependency-embedded
/// plugin is discoverable, registry crates included.
pub async fn discover(
    sym: &Symposium,
    deps: &Arc<WorkspaceDeps>,
    registry: &[Plugin],
) -> Discovery {
    let Some(workspace_root) = deps.workspace_root().map(Path::to_path_buf) else {
        return Discovery::default();
    };
    let pms = sym.package_managers(deps);
    let audits = sym.audit_index(&pms);
    let dep_ids = pms.list_deps().await.unwrap_or_default();
    let used: HashSet<PackageId> =
        resolve_used(sym, &pms, registry, &workspace_root, UpdateLevel::None)
            .await
            .into_iter()
            .map(|(_, id)| id)
            .collect();

    let mut discovery = Discovery::default();
    // Untrusted instances = the cargo transport: its `active_plugins` are the
    // plugins embedded in dependencies, which run only with consent. Classify
    // each against the `[plugins]` config.
    for inst in pms.instances().filter(|i| !i.trusted) {
        for loaded in inst.active_plugins(&dep_ids).await {
            let plugin = loaded.plugin;
            let name = plugin.canonical.name.clone();
            let description = Some(describe_plugin(&plugin.manifest));
            let revocation = audits.revocation(&plugin.canonical, &loaded.root);
            let (enablement, audit) =
                decide(sym, &pms, &used, &plugin.canonical, revocation, || {
                    audits.lookup(&plugin.canonical, &loaded.root)
                });
            let discovered = DiscoveredPlugin {
                registry: inst.name.clone(),
                id: plugin.canonical,
                recommends: name,
                description,
                enablement,
                audit,
            };
            match discovered.enablement {
                Enablement::Used => discovery.active.push(discovered),
                Enablement::AutoEnabled => discovery.auto_enabled.push(discovered),
                Enablement::AutoAccepted => discovery.auto_accepted.push(discovered),
                Enablement::Audited { .. } => discovery.audited.push(discovered),
                Enablement::Declined => discovery.declined.push(discovered),
                Enablement::Revoked(_) => discovery.revoked.push(discovered),
                Enablement::Candidate => discovery.candidates.push(discovered),
            }
        }
    }
    discovery
}

/// The packages whose plugins the user has enabled, to load at sync time.
/// Two sources:
///
/// 1. workspace **dependencies** covered by `[plugins] auto-enable`,
///    `auto-accept = "all"`, or an applicable `use` entry, and
/// 2. packages named by a `use` entry that are **not** dependencies:
///    `cargo agents use <crate>` pulls a plugin in from crates.io whether or
///    not the workspace depends on it.
///
/// `auto-enable` and `auto-accept` intentionally contribute only (1): they are
/// consent for what a dependency you already have carries, not a way to add crates. Declined
/// packages are pruned. A `use` name is resolved through the package managers
/// ([`PmRegistry::identify`]) at `update`: one that names a plugin in
/// `registry` is left out, since that plugin is loaded already and
/// [`used_plugins`] wakes it, and one that no package manager has is skipped.
pub async fn enabled_dependencies(
    sym: &Symposium,
    pms: &PmRegistry,
    registry: &[Plugin],
    dep_ids: &[PackageId],
    workspace_root: &Path,
    update: UpdateLevel,
) -> Vec<PackageId> {
    let plugins = &sym.config.plugins;
    let used = resolve_used(sym, pms, registry, workspace_root, update).await;
    let mut enabled = Vec::new();
    let mut known = HashSet::new();
    let audits = sym.audit_index(pms);
    for id in dep_ids.iter().filter(|id| id.pm == CARGO_PM) {
        let identity = pms.identity(id);
        let is_used = used.iter().any(|(_, used)| *used == identity);
        known.insert(identity);
        if plugins.is_disabled(pms, id) {
            continue;
        }
        if plugins.is_accepted_without_asking(pms, id)
            || is_used
            || is_audited_dependency(sym, pms, &audits, id).await
        {
            enabled.push(PackageId::any_version(&id.pm, &id.name));
        }
    }

    for (name, id) in used {
        let named = PackageId::any_version(&id.pm, name);
        if !registry.iter().any(|p| pms.identity(&p.canonical) == id)
            && !plugins.is_disabled(pms, &named)
            && known.insert(id)
        {
            enabled.push(named);
        }
    }

    enabled
}

/// Under `auto-accept = "audit"`, does a trusted registry audit exactly the
/// payload of the dependency `id`? Cache-only: the source is the one already on
/// disk. Looked up by name first, so a crate nobody audited is neither fetched
/// nor hashed. A revoked payload is not excluded here; activation withholds it
/// (see `plugins::withheld`) exactly as it does for any other grant.
async fn is_audited_dependency(
    sym: &Symposium,
    pms: &PmRegistry,
    audits: &AuditIndex,
    id: &PackageId,
) -> bool {
    if sym.config.plugins.auto_accept != AutoAccept::Audit {
        return false;
    }
    let crate_audits = audits.for_crate(&id.name);
    if !crate_audits.has_audits() {
        return false;
    }
    // Fetched by name, as loading will: the workspace pin (or path override)
    // picks the release.
    let any = PackageId::any_version(&id.pm, &id.name);
    let Ok(fetched) = pms.fetch(&any, UpdateLevel::None).await else {
        return false;
    };
    matches!(
        audits.lookup_in(&crate_audits, &fetched.id, &fetched.root),
        AuditLookup::Audited(_)
    )
}

pub(crate) async fn resolve_used<'a>(
    sym: &'a Symposium,
    pms: &PmRegistry,
    registry: &[Plugin],
    workspace_root: &Path,
    update: UpdateLevel,
) -> Vec<(&'a str, PackageId)> {
    let mut used = Vec::new();
    for name in sym.config.plugins.used_names_in(workspace_root) {
        match pms.identify(registry, name, update).await {
            Ok(id) => used.push((name, id)),
            Err(e) => tracing::warn!(error = %e, "skipping `[plugins] use` entry"),
        }
    }
    used
}

/// The canonical ids of the `plugins` an applicable `use` entry names, which
/// is what wakes a dormant one. A plugin is named by its manifest name, under
/// the rule of the package manager it came from.
pub fn used_plugins(
    sym: &Symposium,
    pms: &PmRegistry,
    plugins: &[Plugin],
    workspace_root: &Path,
) -> Vec<PackageId> {
    plugins
        .iter()
        .filter(|p| {
            let named = PackageId::any_version(&p.canonical.pm, &p.manifest.name);
            sym.config.plugins.is_used_in(pms, &named, workspace_root)
        })
        .map(|p| p.canonical.clone())
        .collect()
}

/// Run [`discover`] for the workspace `deps` points at, against the plugins
/// its registries and members define, or an empty [`Discovery`] when there is
/// no workspace.
pub async fn discover_for(sym: &Symposium, deps: &Arc<WorkspaceDeps>) -> Discovery {
    let workspace = deps.load().cloned();
    let registry = crate::plugins::load_registry_with_workspace(sym, workspace.as_deref()).await;
    discover(sym, deps, &registry.plugins).await
}

/// The names of the discovered offers still awaiting consent, deduplicated
/// and sorted — what a consent prompt would ask about, and what the
/// non-interactive hint names.
pub async fn pending_candidates(sym: &Symposium, deps: &Arc<WorkspaceDeps>) -> Vec<String> {
    let mut names: Vec<String> = discover_for(sym, deps)
        .await
        .candidates
        .into_iter()
        .map(|c| c.id.name)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Record consent decisions: approvals into `[plugins] auto-enable`,
/// declines into `[plugins] disable`, then save the config. Both lists hold
/// dependency crate names, as [`discover`] offers them.
///
/// Split out from the prompt so the decision recording is testable without a
/// terminal, and so other entry points can record the same way.
pub fn apply_consent(sym: &mut Symposium, approved: &[String], declined: &[String]) -> Result<()> {
    if approved.is_empty() && declined.is_empty() {
        return Ok(());
    }
    let pms = sym.detached_managers();
    let plugins = &mut sym.config.plugins;
    for name in approved {
        if !plugins.is_auto_enabled(&pms, &CargoPm::id_for(name, None)) {
            plugins.auto_enable.push(name.clone());
        }
    }
    for name in declined {
        if !plugins.is_disabled(&pms, &CargoPm::id_for(name, None)) {
            plugins.disable.push(name.clone());
        }
    }
    sym.save_config().context("failed to write user config")?;

    if !approved.is_empty() {
        tracing::info!(
            report = %ReportEvent::Info {
                message: format!("enabled dependency plugins: {}", approved.join(", ")),
            },
        );
    }
    if !declined.is_empty() {
        tracing::info!(
            report = %ReportEvent::Info {
                message: format!(
                    "declined dependency plugins (recorded in `[plugins] disable`): {}",
                    declined.join(", ")
                ),
            },
        );
    }
    Ok(())
}

/// Ask the user about each undecided offer, then record the answers.
///
/// **Never prompts unless `out` is interactive** ([`Output::is_interactive`]):
/// hook dispatch and anything an agent triggers use a quiet or capturing
/// output, so they return here immediately without touching stdin. In those
/// contexts the candidates surface as a `SessionStart` hint instead (see
/// [`pending_candidates`]).
///
/// Only explicit answers are recorded — the default ("ask me later") leaves
/// the dependency undecided, so reflexively hitting Enter never permanently
/// declines anything, and Escape leaves the remaining offers undecided too.
pub async fn prompt_for_consent(
    sym: &mut Symposium,
    deps: &Arc<WorkspaceDeps>,
    out: &Output,
) -> Result<()> {
    if !out.is_interactive() {
        return Ok(());
    }
    let candidates = discover_for(sym, deps).await.candidates;
    if candidates.is_empty() {
        return Ok(());
    }

    let mut approved = Vec::new();
    let mut declined = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        let name = candidate.id.name.clone();
        if !seen.insert(name.clone()) {
            continue;
        }
        let answer = dialoguer::Select::new()
            .with_prompt(consent_question(&candidate))
            .items(["Ask me later", "Enable", "No — don't ask again"])
            .default(0)
            .interact_opt()
            .context("consent prompt failed")?;
        match answer {
            Some(1) => approved.push(name),
            Some(2) => declined.push(name),
            Some(_) => {}  // ask me later — record nothing
            None => break, // Esc — leave the rest undecided
        }
    }
    apply_consent(sym, &approved, &declined)
}

/// The question the consent prompt asks about one candidate. It leads with the
/// audit status, since "no registry has audited this" is what the user is
/// being asked to take on trust.
fn consent_question(candidate: &DiscoveredPlugin) -> String {
    let name = candidate.name();
    let what = candidate
        .description
        .as_deref()
        .unwrap_or("agent extensions");
    let status = match &candidate.audit {
        None => "Audit status unknown.".to_string(),
        Some(AuditLookup::Unaudited) => {
            "NOT AUDITED: no trusted registry has audited this plugin.".to_string()
        }
        Some(AuditLookup::Ineligible) => format!(
            "NOT AUDITED: a registry has audited `{name}`, but this plugin's payload cannot be checked against an audit (it fetches skills from outside the crate, or is too large)."
        ),
        Some(AuditLookup::PayloadDiffers) => format!(
            "NOT AUDITED: a registry has audited `{name}`, but that audit does not cover this release's payload."
        ),
        Some(AuditLookup::Audited(registry)) => format!(
            "Audited by registry `{registry}`, but `[plugins] auto-accept` is `none`, so it asks anyway."
        ),
    };
    format!("{status}\nDependency `{name}` provides {what}. Enable it?")
}

/// A short human summary of what a discovered plugin contributes, for the
/// consent prompt and status output. Emphasizes the facets that matter to a
/// trust decision — a plugin that only ships skills is lower-stakes than one
/// that runs a hook or an MCP server.
fn describe_plugin(plugin: &crate::plugins::PluginManifest) -> String {
    let parts: Vec<String> = [
        count_phrase(plugin.skills.len(), "skill group", "skill groups"),
        count_phrase(plugin.hooks.len(), "hook", "hooks"),
        count_phrase(plugin.mcp_servers.len(), "MCP server", "MCP servers"),
        count_phrase(plugin.subcommands.len(), "subcommand", "subcommands"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "agent extensions".to_string()
    } else {
        parts.join(", ")
    }
}

/// `"<n> <singular|plural>"`, or `None` when `n` is zero.
fn count_phrase(n: usize, singular: &str, plural: &str) -> Option<String> {
    (n > 0).then(|| format!("{n} {}", if n == 1 { singular } else { plural }))
}

/// Classify one offer against the `[plugins]` config and the registries'
/// revocations (`revocation`, as [`AuditIndex::revocation`] answered for it),
/// and, for one nobody has decided, their audits (`lookup`, run only then, so
/// a payload is hashed only when the answer can matter). Returns the decision
/// and what the audit lookup found (`None` when the offer was
/// decided before the lookup would run).
///
/// First match wins, in this order:
///
/// 1. `disable` names it: `Declined`;
/// 2. the registries revoke it: `Revoked`;
/// 3. a `use` entry names it: `Used`;
/// 4. an `auto-enable` entry names it: `AutoEnabled`;
/// 5. `auto-accept = "all"` alone: `AutoAccepted`;
/// 6. only when none of those apply, the audit `lookup` under
///    `auto-accept = "audit"`: `Audited`;
/// 7. otherwise `Candidate`.
///
/// So a name the user declined stays declined, a revoked release stays out
/// whatever consented to it, and under `all` an audited plugin is
/// `AutoAccepted`, not `Audited`. Under `auto-accept = "none"` an audit never
/// skips the prompt.
///
/// [`AuditIndex::revocation`]: crate::audits::AuditIndex::revocation
fn decide(
    sym: &Symposium,
    pms: &PmRegistry,
    used: &HashSet<PackageId>,
    id: &PackageId,
    revocation: Option<Revocation>,
    lookup: impl FnOnce() -> AuditLookup,
) -> (Enablement, Option<AuditLookup>) {
    let plugins = &sym.config.plugins;
    // `disable` is tested first because it is the last word: every other
    // entry says a plugin *may* run, and this is the only one that says it
    // may not. Activation agrees (see `record_active`), so classifying a
    // disabled plugin as `Used` here would report it active while it stays
    // off.
    let enablement = if plugins.is_disabled(pms, id) {
        Enablement::Declined
    } else if let Some(revocation) = revocation {
        Enablement::Revoked(revocation)
    } else if used.contains(&pms.identity(id)) {
        Enablement::Used
    } else if plugins.is_auto_enabled(pms, id) {
        // An explicit entry is the more specific reason, so it wins over `all`.
        Enablement::AutoEnabled
    } else if plugins.auto_accept == AutoAccept::All {
        Enablement::AutoAccepted
    } else {
        let audit = lookup();
        return match (&audit, plugins.auto_accept) {
            (AuditLookup::Audited(registry), AutoAccept::Audit) => (
                Enablement::Audited {
                    registry: registry.clone(),
                },
                Some(audit),
            ),
            _ => (Enablement::Candidate, Some(audit)),
        };
    };
    (enablement, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm::ANY_VERSION;
    use crate::pm::WorkspaceCrate;
    use indoc::indoc;

    /// A workspace with `widget-lib` as a path dependency carrying skills,
    /// plus a plain registry dependency (an extracted source with no plugin
    /// content, as `cargo metadata` always yields a `source_dir`).
    fn workspace(root: &Path) -> Arc<WorkspaceDeps> {
        let widget = root.join("widget-lib");
        std::fs::create_dir_all(widget.join("skills/guidance")).unwrap();
        std::fs::write(widget.join("skills/guidance/SKILL.md"), "").unwrap();
        let serde = root.join("serde-src");
        std::fs::create_dir_all(&serde).unwrap();
        WorkspaceDeps::fixture(
            root.to_path_buf(),
            vec![
                WorkspaceCrate::new(
                    "widget-lib".to_string(),
                    semver::Version::new(1, 0, 0),
                    Some(widget),
                ),
                WorkspaceCrate::new("serde".to_string(), semver::Version::new(1, 0, 210), None)
                    .with_source_dir(Some(serde)),
            ],
        )
    }

    /// A `Symposium` over a fresh config dir with only the given config, and
    /// no built-in registries (so tests see only what they set up).
    fn sym_with(root: &Path, config: &str) -> Symposium {
        let config_dir = root.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            format!(
                "{}\n[defaults]\nsymposium-recommendations = false\nuser-plugins = false\n",
                config
            ),
        )
        .unwrap();
        Symposium::from_dir(&config_dir)
    }

    #[tokio::test]
    async fn undecided_dependency_plugin_is_a_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        let sym = sym_with(tmp.path(), "");

        let found = discover(&sym, &ws, &[]).await;
        assert!(found.active.is_empty());
        assert!(found.auto_enabled.is_empty());
        let names: Vec<&str> = found.candidates.iter().map(|c| c.name()).collect();
        assert_eq!(names, vec!["widget-lib"]);
        assert_eq!(found.candidates[0].recommends, "widget-lib");
    }

    #[tokio::test]
    async fn auto_enable_moves_a_candidate_to_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                auto-enable = ["widget_lib"]
            "#},
        );

        let found = discover(&sym, &ws, &[]).await;
        assert!(found.candidates.is_empty());
        let names: Vec<&str> = found.auto_enabled.iter().map(|c| c.name()).collect();
        assert_eq!(names, vec!["widget-lib"]);
        assert!(found.enabled().count() == 1);
    }

    /// A revocation outranks every grant (`use`, `auto-enable`,
    /// `auto-accept = all`) and is outranked only by `disable`.
    #[test]
    fn revocation_outranks_grants_and_only_disable_outranks_it() {
        use crate::config::AutoAccept;
        use symposium_sdk::audits::RevokeReason;

        let tmp = tempfile::tempdir().unwrap();
        let id = CargoPm::id_for("widget-lib", Some("1.0.0"));
        let revocation = || Revocation {
            registry: "reg".into(),
            reason: RevokeReason::Exfiltration,
        };
        let classify = |config: &str, used: bool, accept_all: bool| {
            let mut sym = sym_with(tmp.path(), config);
            if accept_all {
                sym.config.plugins.auto_accept = AutoAccept::All;
            }
            let pms = sym.detached_managers();
            let used: HashSet<PackageId> = used.then(|| pms.identity(&id)).into_iter().collect();
            decide(&sym, &pms, &used, &id, Some(revocation()), || {
                panic!("a decided offer is not looked up")
            })
            .0
        };
        let revoked = Enablement::Revoked(revocation());
        assert_eq!(classify("", false, false), revoked);
        assert_eq!(classify("", true, false), revoked, "use");
        assert_eq!(
            classify("[plugins]\nauto-enable = [\"widget-lib\"]", false, false),
            revoked,
            "auto-enable"
        );
        assert_eq!(classify("", false, true), revoked, "auto-accept = all");
        assert_eq!(
            classify("[plugins]\ndisable = [\"widget_lib\"]", true, true),
            Enablement::Declined
        );
        assert!(!revoked.is_enabled());
    }

    #[tokio::test]
    async fn use_entry_and_disable_outrank_the_standing_decisions() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());

        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                use = ["widget-lib"]
            "#},
        );
        let found = discover(&sym, &ws, &[]).await;
        assert_eq!(found.active.len(), 1);
        assert_eq!(found.active[0].enablement, Enablement::Used);

        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                auto-enable = ["*"]
                disable = ["widget-lib"]
            "#},
        );
        let found = discover(&sym, &ws, &[]).await;
        assert!(found.auto_enabled.is_empty());
        assert_eq!(found.declined.len(), 1);
    }

    #[tokio::test]
    async fn auto_accept_modes_decide_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        // No registry audits anything here, so `audit` asks like `none`.
        for mode in ["none", "audit"] {
            let sym = sym_with(
                tmp.path(),
                &format!("[plugins]\nauto-accept = \"{mode}\"\n"),
            );
            let found = discover(&sym, &ws, &[]).await;
            assert_eq!(found.candidates.len(), 1, "{mode}");
            assert!(found.auto_accepted.is_empty(), "{mode}");
        }
        let sym = sym_with(tmp.path(), "[plugins]\nauto-accept = \"all\"\n");
        let found = discover(&sym, &ws, &[]).await;
        assert!(found.candidates.is_empty());
        assert!(found.auto_enabled.is_empty());
        assert_eq!(found.auto_accepted.len(), 1);
        assert_eq!(found.auto_accepted[0].enablement, Enablement::AutoAccepted);
        // Sync loads the crate.
        let deps = [PackageId::new(CARGO_PM, "widget-lib", "1.0.0")];
        assert_eq!(
            enabled_names(&sym, &deps, tmp.path()).await,
            vec!["widget-lib".to_string()]
        );
    }

    #[tokio::test]
    async fn auto_accept_all_yields_to_disable_and_use() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        let sym = sym_with(
            tmp.path(),
            "[plugins]\nauto-accept = \"all\"\ndisable = [\"widget-lib\"]\n",
        );
        let found = discover(&sym, &ws, &[]).await;
        assert_eq!(found.declined.len(), 1);
        assert!(found.auto_enabled.is_empty());
        let deps = [PackageId::new(CARGO_PM, "widget-lib", "1.0.0")];
        assert!(enabled_names(&sym, &deps, tmp.path()).await.is_empty());

        let sym = sym_with(
            tmp.path(),
            "[plugins]\nauto-accept = \"all\"\nuse = [\"widget-lib\"]\n",
        );
        let found = discover(&sym, &ws, &[]).await;
        assert_eq!(found.active.len(), 1);
        assert_eq!(found.active[0].enablement, Enablement::Used);
    }

    #[tokio::test]
    async fn use_naming_a_registry_plugin_leaves_the_dependency_undecided() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                use = ["widget-lib"]

                [[registry]]
                name = "reg"
                path = "reg"
            "#},
        );
        let entry = tmp.path().join("config/reg/widget-lib");
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("Symposium.toml"), "name = \"widget-lib\"\n").unwrap();
        let registry = crate::plugins::load_registry(&sym).await;

        let found = discover(&sym, &ws, &registry.plugins).await;
        assert!(found.active.is_empty());
        assert_eq!(found.candidates.len(), 1);
    }

    /// A `use` entry recorded for another workspace does not enable anything
    /// here.
    #[tokio::test]
    async fn workspace_scoped_use_entries_only_count_in_their_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                use = [{ name = "widget-lib", workspace = "/elsewhere" }]
            "#},
        );

        let found = discover(&sym, &ws, &[]).await;
        assert!(found.active.is_empty());
        assert_eq!(found.candidates.len(), 1);
    }

    /// The crates the tests below name, all cached: every dependency they
    /// list, plus `my-skills-crate`, which no workspace depends on.
    fn cached_crates() -> PmRegistry {
        let (cargo, _) = crate::test_utils::FakeCargo::instance(
            &["serde", "tokio", "clap", "anyhow", "my-skills-crate"],
            &[],
        );
        PmRegistry::new(vec![cargo])
    }

    async fn enabled_names(sym: &Symposium, deps: &[PackageId], root: &Path) -> Vec<String> {
        enabled_dependencies(sym, &cached_crates(), &[], deps, root, UpdateLevel::None)
            .await
            .into_iter()
            .map(|id| id.name)
            .collect()
    }

    /// Hook dispatch, `--help` and subcommand dispatch build the active set at
    /// `UpdateLevel::None`, so a `use` entry for a crate that is neither a
    /// dependency nor cached is skipped without a crates.io search or a cargo
    /// process; `sync` resolves it at `Check`.
    #[tokio::test]
    async fn resolving_use_entries_without_update_stays_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                use = ["published-crate"]
            "#},
        );
        let registry = crate::plugins::load_registry(&sym).await;
        let (cargo, network) = crate::test_utils::FakeCargo::instance(&[], &["published-crate"]);
        let pms = PmRegistry::new(vec![cargo]);
        let active_at = async |update| {
            let mut ctx = crate::predicate::PredicateContext::new(&[]);
            crate::plugins::active_plugins(
                &sym,
                &registry,
                &pms,
                Some(tmp.path()),
                &mut ctx,
                update,
            )
            .await
        };

        active_at(UpdateLevel::None).await;
        assert_eq!(network.load(std::sync::atomic::Ordering::SeqCst), 0);

        active_at(UpdateLevel::Check).await;
        assert_eq!(network.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn enabled_dependencies_reads_config_not_offers() {
        let tmp = tempfile::tempdir().unwrap();
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                auto-enable = ["serde"]
                use = ["tokio"]
                disable = ["clap"]
            "#},
        );
        let deps = [
            // A registry dependency, invisible to `active_plugins`, is still
            // enabled by name.
            PackageId::new(CARGO_PM, "serde", "1.0.210"),
            PackageId::new(CARGO_PM, "tokio", "1.0.0"),
            PackageId::new(CARGO_PM, "clap", "4.0.0"),
            PackageId::new(CARGO_PM, "anyhow", "1.0.0"),
            // Not a cargo package: not a crate to load.
            PackageId::new("npm", "serde", ANY_VERSION),
        ];

        assert_eq!(
            enabled_names(&sym, &deps, tmp.path()).await,
            vec!["serde".to_string(), "tokio".to_string()]
        );
    }

    /// `use`-ing a crate that isn't a workspace dependency still enables it, so
    /// sync loads its plugin from the registry. `auto-enable` does not: it is
    /// consent for dependencies you already have. A `use` name no package
    /// manager has is skipped.
    #[tokio::test]
    async fn use_enables_a_non_dependency_crate_but_auto_enable_does_not() {
        let tmp = tempfile::tempdir().unwrap();
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                auto-enable = ["not-a-dep-autoenable"]
                use = ["my-skills-crate", "ghost", { name = "scoped-crate", workspace = "/elsewhere" }]
            "#},
        );
        // Only `anyhow` is an actual dependency; the rest are not.
        let deps = [PackageId::new(CARGO_PM, "anyhow", "1.0.0")];

        // The `use`d non-dependency crate is enabled; the workspace-scoped one
        // (for /elsewhere), the unknown one and the auto-enabled
        // non-dependency are not.
        assert_eq!(
            enabled_names(&sym, &deps, tmp.path()).await,
            vec!["my-skills-crate".to_string()]
        );
    }

    /// A `use`d crate that *is* a dependency appears once, not twice.
    #[tokio::test]
    async fn used_dependency_is_not_duplicated() {
        let tmp = tempfile::tempdir().unwrap();
        let sym = sym_with(
            tmp.path(),
            indoc! {r#"
                [plugins]
                use = ["serde"]
            "#},
        );
        let deps = [PackageId::new(CARGO_PM, "serde", "1.0.210")];
        assert_eq!(
            enabled_names(&sym, &deps, tmp.path()).await,
            vec!["serde".to_string()]
        );
    }

    /// A registry compares plugin names exactly, so only the exact spelling
    /// in `use` wakes a dormant registry plugin.
    #[tokio::test]
    async fn use_wakes_a_registry_plugin_only_by_its_exact_name() {
        let used = async |entry: &str| {
            let tmp = tempfile::tempdir().unwrap();
            let config = format!(
                "[plugins]\nuse = [\"{entry}\"]\n\n[[registry]]\nname = \"reg\"\npath = \"reg\"\n"
            );
            let sym = sym_with(tmp.path(), &config);
            let entry_dir = tmp.path().join("config/reg/my-plugin");
            std::fs::create_dir_all(&entry_dir).unwrap();
            std::fs::write(entry_dir.join("Symposium.toml"), "name = \"my-plugin\"\n").unwrap();
            let plugins = crate::plugins::load_registry(&sym).await.plugins;
            assert_eq!(plugins.len(), 1);
            assert!(plugins[0].manifest.requires_use);
            used_plugins(&sym, &sym.detached_managers(), &plugins, tmp.path())
                == [plugins[0].canonical.clone()]
        };

        assert!(used("my-plugin").await);
        assert!(!used("my_plugin").await);
    }

    fn candidate_with(audit: AuditLookup) -> DiscoveredPlugin {
        DiscoveredPlugin {
            registry: CARGO_PM.into(),
            id: CargoPm::id_for("widget-lib", Some("1.0.0")),
            recommends: "widget-lib".into(),
            description: Some("1 skill group".into()),
            enablement: Enablement::Candidate,
            audit: Some(audit),
        }
    }

    #[test]
    fn the_prompt_says_prominently_when_a_plugin_is_not_audited() {
        let q = consent_question(&candidate_with(AuditLookup::Unaudited));
        assert!(q.starts_with("NOT AUDITED: no trusted registry"), "{q}");
        assert!(q.contains("Dependency `widget-lib` provides 1 skill group. Enable it?"));

        let q = consent_question(&candidate_with(AuditLookup::Ineligible));
        assert!(q.starts_with("NOT AUDITED: a registry has audited"), "{q}");
        assert!(q.contains("cannot be checked against an audit"), "{q}");

        let q = consent_question(&candidate_with(AuditLookup::PayloadDiffers));
        assert!(q.starts_with("NOT AUDITED"), "{q}");
        assert!(q.contains("does not cover this release's payload"), "{q}");

        let mut unlooked = candidate_with(AuditLookup::Unaudited);
        unlooked.audit = None;
        assert!(consent_question(&unlooked).starts_with("Audit status unknown."));

        let q = consent_question(&candidate_with(AuditLookup::Audited("reg".into())));
        assert!(!q.contains("NOT AUDITED"), "{q}");
        assert!(q.contains("`reg`") && q.contains("auto-accept"), "{q}");
    }
}
