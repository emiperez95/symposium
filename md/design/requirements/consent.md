# Consent requirements

A plugin embedded in a workspace dependency runs only with the user's consent: dependencies are not a trust root. These requirements say what counts as consent and how it is recorded. An *applicable* `[plugins] use` entry is an unscoped (`--global`) entry, or one recorded for the current workspace root. Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} unconsented_inactive
Symposium MUST NOT activate a plugin embedded in a workspace dependency unless `[plugins] auto-enable` or an applicable `[plugins] use` entry names that dependency.
:::

*Source:* Documented in md/reference/configuration.md:131 "a plugin embedded in a dependency runs only once you say so".

:::{req} consented_active
Symposium MUST activate the plugin embedded in a workspace dependency that `[plugins] auto-enable` or an applicable `[plugins] use` entry names, when `[plugins] disable` does not name it.
:::

*Source:* Documented in md/reference/configuration.md:137 "`auto-enable` ... Dependency names whose embedded plugins load without being asked about"; md/reference/cargo-agents-use.md:38-41 "`use` is the by-name way to say so".

:::{req} disable_wins
Symposium MUST NOT activate a dependency plugin or `use`d crate that `[plugins] disable` names, even when `[plugins] auto-enable` or a `[plugins] use` entry also names it.
:::

*Source:* Documented in md/reference/configuration.md:139 "Names that must never be enabled. Takes precedence over `auto-enable`, including over `\"*\"`"; tests/enablement.rs:764-766 "A plugin named by both `use` and `disable` is reported declined, matching what activation does with it".

*Note:* `disable` does not block registry plugins (trust roots) or crates a registry plugin chains to. Nothing in `plugins::active_plugins` / `record_active` checks it. The configuration doc's "must never be enabled" reads broader than the code.

:::{req} scoped_use
A `[plugins] use` entry recorded for one workspace MUST NOT enable anything while Symposium works in another workspace.
:::

*Source:* Documented in md/reference/configuration.md:138 "`{ name = \"...\", workspace = \"/path\" }` (enabled only while working in that workspace root)".

:::{req} use_beyond_dependencies
An applicable `[plugins] use` entry that names a crate the workspace does not depend on MUST cause Symposium to load that crate's plugin.
:::

*Source:* Documented in md/reference/configuration.md:150 "it also enables a plugin whether or not any dependency references it".

:::{req} auto_enable_dependencies_only
An `[plugins] auto-enable` entry MUST NOT enable anything other than a plugin embedded in a workspace dependency.
:::

*Source:* Documented in md/reference/configuration.md:150 "`auto-enable` is narrower: it is consent for what a dependency you already have carries with it"; tests/enablement.rs:244-245 "`auto-enable` is consent for dependencies, so it wakes no registry plugin whatever the spelling".

:::{req} crate_name_matching
Symposium MUST treat hyphens and underscores as equal when it compares a `[plugins]` or `cargo agents use` name with a cargo crate name.
:::

*Source:* Documented in md/reference/configuration.md:141 "a crate name the way cargo matches it, hyphen/underscore-insensitively".

:::{req} registry_name_exact
Symposium MUST compare a `[plugins]` or `cargo agents use` name with a registry plugin's name exactly.
:::

*Source:* Documented in md/reference/configuration.md:141 "and a registry plugin's name exactly".

:::{req} use_resolves_registry_first
When a `[plugins] use` name matches both a configured registry plugin and a crate, Symposium MUST resolve it to the registry plugin.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:56 "When a registry plugin and a crate share the name, the registry plugin wins.".

:::{req} registry_use_not_dependency_consent
A `[plugins] use` entry that resolves to a registry plugin MUST NOT count as consent for the plugin embedded in a same-named dependency.
:::

*Source:* Documented in md/reference/configuration.md:141 "means the registry plugin only, so it does not also consent to that crate's embedded plugin".

:::{req} dormant_inactive
Symposium MUST NOT activate a registry plugin that no dependency gates unless an applicable `[plugins] use` entry names it.
:::

*Source:* Documented in md/reference/configuration.md:131 "a registry plugin that names no dependency anywhere is *dormant* — loaded and listed, but inactive — until you enable it by name".

:::{req} use_wakes_dormant
Symposium MUST activate a dormant registry plugin while an applicable `[plugins] use` entry names it.
:::

*Source:* Documented in md/reference/configuration.md:150 "`use` is what wakes a dormant plugin".

:::{req} offline_resolution
When Symposium resolves `[plugins] use` entries at `UpdateLevel::None` (hook dispatch, `--help`, subcommand dispatch, `status`, discovery), it MUST NOT search a remote registry or spawn a package-manager process.
:::

*Source:* Documented in src/discovery.rs:589-592 (test doc) "a `use` entry for a crate that is neither a dependency nor cached is skipped without a crates.io search or a cargo process"; md/design/module-structure.md:185 "Status resolves names at `None`, so it never searches crates.io".

:::{req} unresolved_use_skipped
Symposium MUST skip, without failing, a `[plugins] use` entry that no package manager resolves.
:::

*Source:* Documented in md/reference/configuration.md:141 "a `use` name no package manager has enables nothing"; md/design/module-structure.md:171 "one no package manager has is skipped with a warning in the log".

:::{req} enabled_once
The list of consented packages Symposium loads MUST name each package at most once.
:::

*Source:* Documented in src/discovery.rs:682 (test doc) "A `use`d crate that *is* a dependency appears once, not twice.".

*Note:* this is an internal invariant. `plugins::active_plugins`'s `visited` set already collapses duplicates downstream.

:::{req} prompt_once_per_candidate
An interactive consent prompt MUST ask once about each dependency plugin awaiting consent.
:::

*Source:* Documented in md/design/module-structure.md:173 "`prompt_for_consent(sym, deps, out)` asks one question per candidate"; md/reference/cargo-agents-sync.md:33-34 "asks about each dependency whose source embeds an agent plugin that you have not decided about yet".

:::{req} prompt_describes_extensions
An interactive consent prompt SHOULD say which kinds of extension the plugin contributes: skill groups, hooks, MCP servers, subcommands.
:::

*Source:* Documented in src/discovery.rs:367-370 (doc comment) "A short human summary of what a discovered plugin contributes, for the consent prompt ... Emphasizes the facets that matter to a trust decision".

:::{req} approval_recorded
Symposium MUST record a consent approval as a `[plugins] auto-enable` entry.
:::

*Source:* Documented in md/reference/cargo-agents-sync.md:39 "**Enable** — recorded in `[plugins] auto-enable`".

:::{req} decline_recorded
Symposium MUST record a consent decline as a `[plugins] disable` entry.
:::

*Source:* Documented in md/reference/cargo-agents-sync.md:40 "**No — don't ask again** — recorded in `[plugins] disable`".

:::{req} deferral_records_nothing
Symposium MUST record nothing for a consent prompt answered with the default ("Ask me later") or cancelled.
:::

*Source:* Documented in md/reference/cargo-agents-sync.md:38 "**Ask me later** (the default) — records nothing"; :42-43 "Escape leaves the remaining questions undecided".
