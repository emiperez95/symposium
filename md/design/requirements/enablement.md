# Enablement command requirements

What `cargo agents use`, `cargo agents search` and `cargo agents status` promise. The rules they apply are in [Consent requirements](./consent.md). Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} status_once
`cargo agents status` MUST list each plugin it reports on one line.
:::

*Source:* Documented in md/reference/cargo-agents-status.md:17 "`status` reports both, one line per plugin".

*Note:* two `disable` spellings that no package manager knows (`never-seen` / `never_seen`) are listed twice, by design (`status_lists_each_declined_name_once`).

:::{req} status_state
`cargo agents status` MUST give each plugin the state that activation gives it: active, dormant, candidate (awaiting consent), or declined.
:::

*Source:* Documented in md/reference/cargo-agents-status.md:20-27 (state table); tests/enablement.rs:765-766 "The two must agree, or `status` says a plugin is active while it never runs.".

:::{req} use_scope
`cargo agents use <name>` MUST record its entry for the current workspace, or unscoped when `--global` is given.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:17 "`--global` | Enable for every workspace instead of just the current one"; md/design/module-structure.md:181 "workspace-scoped by default, `Global` with `--global`".

:::{req} use_idempotent
`cargo agents use <name>` MUST NOT add an entry when one in the requested scope already names the same package.
:::

*Source:* Documented in tests/enablement.rs:69-71 (test doc) "`use <dep>` records a workspace-scoped entry ... running it again changes nothing."; md/design/module-structure.md:181 "Both compare existing entries by the identity each resolves to".

:::{req} use_registry_noop
`cargo agents use <name>` MUST record nothing when a configured registry already activates the named plugin.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:46-48 "Anything a configured registry already offers under a `depends-on` gate is enabled by configuration ... so `use`-ing it is a no-op".

:::{req} use_unknown_rejected
`cargo agents use <name>` MUST fail without changing the config when `<name>` resolves to no package.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:57 "Otherwise the command fails with `no package named <NAME>` and writes nothing.".

:::{req} use_disabled_rejected
`cargo agents use <name>` MUST fail without changing the config when `[plugins] disable` names the package.
:::

*Source:* Documented in tests/enablement.rs:484-486 (test doc) "`disable` is the last word, so `use` on a declined plugin is refused rather than recorded.".

*Note:* md/reference/cargo-agents-use.md does not mention this refusal. Worth adding a line there.

:::{req} use_syncs
`cargo agents use` and `cargo agents use --remove` MUST sync the workspace after changing the config.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:3 "Enable a plugin by name, and sync it into the workspace immediately."; :64-65 "The sync that follows reaps the plugin's installed skills.".

:::{req} remove_requires_entry
`cargo agents use --remove <name>` MUST fail without changing the config when no entry in the requested scope names `<name>`.
:::

*Source:* Documented in md/reference/cargo-agents-use.md:62-64 "A scope mismatch (or no entry at all) is an error rather than a silent success.".

:::{req} search_matches
`cargo agents search <query>` MUST list every configured-registry plugin whose name contains `<query>`, ignoring case.
:::

*Source:* Documented in md/reference/cargo-agents-search.md:19 "The query is a case-insensitive substring match"; :23-26 "plugin names in the plugin registry".

:::{req} search_once
`cargo agents search <query>` MUST list each matching registry entry at most once.
:::

*Source:* Documented in md/reference/cargo-agents-search.md:28-29 "An entry the first arm already listed under the same name is not listed again.".

:::{req} search_origin
`cargo agents search <query>` MUST tag each hit with the registry it comes from.
:::

*Source:* Documented in md/reference/cargo-agents-search.md:20-21 "printed grouped by the instance each hit came from".
