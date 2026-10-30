# `cargo agents status`

Show which plugins are enabled for this workspace, and why.

## Usage

```bash
cargo agents status
```

Must be run from within a Rust workspace.

## Behavior

Symposium separates two questions. *Enablement* asks whether a plugin may run
at all; [*activation predicates*](./predicates.md) ask when it applies.
`status` reports both, one line per plugin, each naming its **enablement
root** — so it answers "why is this here?" with "enabled via `serde`".

Each line is in one of five states:

| State | Meaning |
|-------|---------|
| `active` | Enabled and its predicates hold here. The root names the trust root: workspace membership, a configured registry, `[plugins] auto-enable`, `[plugins] auto-accept` (when `all` accepted it and no `auto-enable` entry names it), a `[plugins] use` entry, or a registry's [audit](./audits.md#audited-loading) of exactly this payload (`audited by registry <name>`). |
| `dormant` | Loaded but contributing nothing: a registry plugin awaiting [`cargo agents use`](./cargo-agents-use.md), one whose predicates don't currently hold, a dependency named by `use` that offers no plugin, or a `use` entry naming no registry plugin, dependency or cached crate (a [`cargo agents sync`](./cargo-agents-sync.md) downloads a crate that is not cached yet). |
| `candidate` | Discovered in a dependency and awaiting consent (not audited, or audited only for a different payload). These are exactly what an interactive [`cargo agents sync`](./cargo-agents-sync.md) asks about. |
| `declined` | Recorded in `[plugins] disable` — the record of pruned plugins and declined discoveries. |
| `revoked` | A registry's [audits file](./audits.md) revokes this release or payload of a crate, for the reason shown. It is not loaded, whatever `use`, `auto-enable` or `auto-accept` say. `disable` outranks it: a crate that is both shows as `declined`. |

Discovery is cache-only, so a dependency whose source has not been fetched yet
is simply not listed as a candidate. Enabling it by name still works.

A revoked crate that is only reached through another plugin's `[[plugins]]` chain, not named by a dependency or a `use` entry, is not listed.

With `--json`, each line is emitted as a `plugin_status` event carrying
`name`, `state` (`active`, `dormant`, `candidate`, `declined` or `revoked`), `root`, and — for a discovered dependency plugin — the
resolved `version`.

## Example

```bash
$ cargo agents status
✅ my-tool — workspace member
✅ serde-skills 1.0.0 — `[plugins] use`
✅ audited-crate 0.4.0 — audited by registry `symposium-audits`
💤 team-conventions — registry `user-plugins` (dormant: awaiting `cargo agents use`)
❓ widget-lib 0.3.1 — found via dependency `widget-lib`, awaiting consent (`cargo agents use widget-lib`)
➖ noisy-crate — declined (`[plugins] disable`)
⛔ risky-crate 2.0.1 — revoked by registry `symposium-audits`: it tells the agent to fetch and run remote code
```
