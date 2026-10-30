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

## Payload digest

An audit covers the *plugin payload* of a crate, not its code: the manifest that says what the crate contributes, and the skill content that manifest names. A release that changes only `src/` has the same payload and the same digest, so one audit covers it. [`cargo agents plugin digest`](./cargo-agents-plugin.md#cargo-agents-plugin-digest) prints the digest of a crate or a directory.

The payload is these entries:

1. `Symposium.toml` at the crate root, byte for byte, when there is one.
2. `[package.metadata.symposium]` from `Cargo.toml`, when there is one, as compact JSON with sorted keys (TOML datetimes as strings). Nothing else in `Cargo.toml` is part of the payload.
3. Every file, of any type, in each *skill directory* the manifest names inside the crate. The manifest names them with its `[[skills]]` groups that set `source.path`, plus the default `skills/` group unless `[defaults] skills = false`. A skill directory is one that contains a `SKILL.md`, and everything beneath it counts, so a `SKILL.md` that points at a file in `resources/` is covered together with that file. A file reachable through two groups counts once. Group `predicates` are not consulted, so the digest does not depend on the workspace it is computed in.

Entries are named by their path relative to the crate root, with `/` separators, and sorted bytewise by name. The digest is SHA-256 over the prefix `symposium-payload-v1\0` followed, for each entry, by the name length as 8 bytes big-endian, the name, the content length as 8 bytes big-endian, and the content. Bytes are hashed as published: line endings are not normalized. Symlinks are followed, and the entry is named by the link's path.

### When there is no digest

Some payloads cannot be pinned, so they have no digest and an audit cannot match them:

- a `[[skills]]` group with `source.git` (what it fetches can change under the same crate release);
- a skill path that leaves the crate directory;
- a broken symlink, or a directory symlink that loops back, inside a skill directory;
- an entry in a skill directory that is neither a file nor a directory, or a path that is not UTF-8;
- a manifest that does not parse as a plugin manifest;
- more than 4 MiB of payload content.

`cargo agents plugin digest` prints which of these applies.

## Revocations

A revocation stops a crate's plugin from loading. It is checked every time the active plugins are resolved (sync, hooks, subcommands), against the crate version that resolved and, when a `digest` revocation exists, the payload digest of the crate's source.

- It applies to **cargo-sourced plugins** only: a plugin embedded in a dependency, and a crate reached through a `[[plugins]]` chained reference. It does not apply to registry plugins or workspace plugins, which are content you chose to trust.
- It outranks every grant: [`[plugins] use`](./configuration.md#plugins), `auto-enable`, and `auto-accept = "all"` do not load a revoked crate.
- [`[plugins] disable`](./configuration.md#plugins) outranks it. A crate that is both revoked and disabled shows as declined.
- A payload with no [digest](#when-there-is-no-digest) cannot match a `digest` revocation; a `versions` revocation still applies to it.
- A `versions` requirement is matched against the resolved version only when that is valid semver.
- The check never prompts and never prints. A revoked plugin is skipped quietly, noted in the log, and shown by [`cargo agents status`](./cargo-agents-status.md) as `revoked`, with the registry and the reason. A revoked crate that is reached only deep in a chain of references is not listed there; it shows in the debug trace (`-v`).

Audits themselves do not yet change what loads: an audited payload is still asked about like any other.

## How Symposium reads it

Symposium is lenient where the tooling that maintains a list is strict:

- A malformed entry is dropped with a warning and the rest of the file is used: a digest that is not `sha256:` plus 64 lowercase hex characters, an entry that sets both or neither of `versions` and `digest`, or a requirement that does not parse.
- An `audit` or `revoked` key that is present but not an array of tables (`[revoked]` instead of `[[revoked]]`, or `revoked = "x"`) is dropped with a warning that says so, so a missing bracket cannot silently lose revocations. The rest of the file is still used.
- A file whose `format` is not one Symposium knows is ignored whole, with a warning: it contributes no audits and no revocations.
- Unknown keys are ignored.
- Warnings go to the log, never to standard output, because the same reader runs inside agent hooks.

An audit or a revocation in *any* registry counts, and Symposium remembers which registry each came from. A registry that is unreachable, or whose content has not been fetched yet, contributes nothing.
