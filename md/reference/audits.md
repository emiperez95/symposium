# Audits file

A [registry](./plugin-source.md) can carry **audits**: statements about the plugin content that a crate ships. An audit says "this exact payload was reviewed"; a revocation says "this release, or this payload, must not load".

Audits are read from every *trusted* registry: a `[[registry]]` entry (git or `path`), the built-in recommendations registry, and `~/.symposium/plugins/`. A company can therefore publish its own audits as a plain directory, without a repository.

## Location

One file per crate, in the registry's root-level `audits/` directory:

```text
my-registry/
  audits/
    widget_lib.toml       # the crate `widget-lib`
    serde_json.toml
  my-plugin/
    Symposium.toml
```

The file name is the crate name lowercased with `-` replaced by `_`, plus `.toml`. Looking up a crate reads only that one file in each registry.

The `audits/` directory is reserved: Symposium never looks in it for plugins, so a `SKILL.md` or `Symposium.toml` inside it does not become one. `audits/` directories deeper in the tree are ordinary directories.

## Format

```toml
format = 1
crate  = "widget-lib"

[[audit]]
digest   = "sha256:<64 lowercase hex characters>"
versions = ["1.4.2", "1.4.3"]

[[revoked]]
versions = ">=1.3.0, <1.4.2"
reason   = "remote-exec"

[[revoked]]
digest = "sha256:<64 lowercase hex characters>"
reason = "injection-bait"
```

| Key | Description |
|-----|-------------|
| `format` | The schema version. This page describes `1`. |
| `crate` | The crate name. The file is ignored when this is not the crate its file name says. |
| `[[audit]]` | One entry per distinct *payload* that was audited. |
| `[[audit]].digest` | The [payload digest](./cargo-agents-plugin.md#cargo-agents-plugin-digest) of the audited payload. This is what a crate is matched on. |
| `[[audit]].versions` | The releases known to carry that payload. Bookkeeping for people and tooling: Symposium never matches on it. |
| `[[revoked]]` | A release or payload that must not load. Set **exactly one** of `versions` and `digest`. |
| `[[revoked]].versions` | A [semver requirement](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) such as `>=1.3.0, <1.4.2`, matched against the resolved crate version. A version that is not valid semver never matches. As in cargo, a pre-release version matches only a requirement that itself names a pre-release. |
| `[[revoked]].digest` | A payload digest. Revokes every release carrying that payload. |
| `[[revoked]].reason` | Why, as one of the codes below. |

### Reasons

`reason` is a code, never prose, so Symposium renders the sentence itself (for example in [`cargo agents status`](./cargo-agents-status.md)).

| Code | Rendered as |
|------|-------------|
| `remote-exec` | tells the agent to fetch and run remote code |
| `exfiltration` | tries to send local data to a third party |
| `concealment` | hides what it does from the user |
| `off-task` | steers the agent away from the user's task |
| `injection-bait` | contains text meant to manipulate the agent |
| `advisory` | is covered by a security advisory |
| `report` | was reported as unsafe |

A code this version of Symposium does not know (one a newer release added) is read as unknown and rendered generically; the revocation still applies.

## How Symposium reads it

Symposium is lenient where the tooling that maintains a list is strict:

- A malformed entry is dropped with a warning and the rest of the file is used: a digest that is not `sha256:` plus 64 lowercase hex characters, an entry that sets both or neither of `versions` and `digest`, or a requirement that does not parse.
- A file whose `format` is not one Symposium knows is ignored whole, with a warning: it contributes no audits and no revocations.
- Unknown keys are ignored.
- Warnings go to the log, never to standard output, because the same reader runs inside agent hooks.

An audit or a revocation in *any* registry counts, and Symposium remembers which registry each came from. Where a registry or its content is unreachable, nothing is read from it.
