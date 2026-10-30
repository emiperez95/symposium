# `cargo agents plugin`

Manage plugin sources.

## Usage

```bash
cargo agents plugin <SUBCOMMAND>
```

## Subcommands

### `cargo agents plugin sync`

```bash
cargo agents plugin sync [PROVIDER]
```

Fetch or update git-based plugin sources. If a provider name is given, syncs only that provider (ignoring `auto-update` settings). If omitted, syncs all providers that have `auto-update = true`.

### `cargo agents plugin list`

```bash
cargo agents plugin list
```

List all configured plugin sources and the plugins they provide.

### `cargo agents plugin show`

```bash
cargo agents plugin show <PLUGIN>
```

Show a plugin: its id and its effective configuration as TOML, after validation has filled in defaults and promoted inline installations.

### `cargo agents plugin digest`

```bash
cargo agents plugin digest <TARGET>
```

Print the [payload digest](./audits.md#payload-digest) of a crate: the value an [audits file](./audits.md) matches the crate on. `<TARGET>` is a crate's source directory when it names an existing directory (relative to the working directory), and otherwise a crate name, resolved the way [`cargo agents crate-info`](./cargo-agents-crate-info.md) resolves it (a workspace member's own directory, or the source cargo fetches).

On success the command prints the digest and nothing else (`sha256:` followed by 64 hex characters), so it can be used in a script. When the payload has no digest it prints the reason and exits non-zero. With `--json` it emits a `payload_digest` event carrying `target`, `digest` or `ineligible`, and, for a crate name, `krate` (the crate and the version it resolved to).

Use it to check a crate before adding it to an audits file, or from tooling that maintains one, so that the tooling hashes exactly as Symposium does.

### `cargo agents plugin validate`

```bash
cargo agents plugin validate <PATH> [--no-check-crates]
```

Validate a plugin source directory or a single TOML manifest file. Useful when authoring plugins.

| Flag | Description |
|------|-------------|
| `<PATH>` | Path to a directory or a single `.toml` file |
| `--no-check-crates` | Skip checking that crate names in predicates exist on crates.io |
