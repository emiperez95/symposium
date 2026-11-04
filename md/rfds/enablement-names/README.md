# Clearer enablement names

## TL;DR

- `[plugins]` holds three per-name decisions, and their names do not say what they do: `use` (always load), `auto-enable` (load without asking when a dependency carries it) and `disable` (never load).
- Rename the first two so the condition is in the name: `enable-always` and `enable-if-dependency`. `disable` stays.
- Give each decision a command: `cargo agents enable`, `cargo agents disable`, and `cargo agents reset` to undo one. `cargo agents use` becomes a hidden alias of `enable`.
- Behavior does not change. Old spellings keep working, with a warning.

## Motivation

`use` and `auto-enable` differ in the one thing that matters, whether the crate has to be a dependency, and neither name says so. A user has to read the reference to tell them apart, and the words are harder still for non-native English speakers.

The commands have the same problem. `cargo agents use` writes only `use` entries. Nothing writes `disable` except the consent prompt, and nothing undoes a decision the prompt recorded short of editing the config by hand.

## Change in a nutshell

Before:

```toml
[plugins]
use         = ["team-tools", { name = "crate-a", workspace = "/home/me/project" }]
auto-enable = ["widget-lib"]
disable     = ["noisy-crate"]
```

After:

```toml
[plugins]
enable-always        = ["team-tools", { name = "crate-a", workspace = "/home/me/project" }]
enable-if-dependency = ["widget-lib"]
disable              = ["noisy-crate"]
```

| Key | Meaning | Written by |
|---|---|---|
| `enable-always` | Load it, whether or not you depend on it | `cargo agents enable` |
| `enable-if-dependency` | Load it without asking when a dependency carries it | "enable" in the consent prompt |
| `disable` | Never load it | `cargo agents disable`, "never ask again" in the prompt |

## Detailed plans

### Config keys

The meanings are exactly today's. `enable-always` keeps `use`'s entry forms, a plain name for every workspace or `{ name, workspace }` for one, and still wakes a dormant registry plugin. `enable-if-dependency` keeps `"*"` for every dependency.

`use` and `auto-enable` are still read, as the new keys, with a warning that names the new spelling; when a config has both spellings, their entries are merged. Symposium writes only the new names. A binary older than this change rejects the new keys, so the reference says to remove them before running an older one.

### Commands

- `cargo agents enable <name> [--global]`: what `use` does today. The name must resolve to a registry plugin, a dependency or a crate on crates.io; the entry is scoped to this workspace unless `--global`; then it syncs.
- `cargo agents disable <name>`: adds the name to `disable` and syncs, which removes its installed skills. The name must resolve, so a typo does not silently do nothing.
- `cargo agents reset <name>`: removes every entry naming it that applies here (global or for this workspace) from all three lists, then syncs, so Symposium asks again. Fails when nothing matched. This replaces `use --remove`.

`cargo agents use` stays for one release as a hidden alias: `use <name>` runs `enable`, `use <name> --remove` runs `reset`. The `SessionStart` hint, `cargo agents status` and the docs use the new names.

## Frequently asked questions

### Why keep two kinds of enable?

Say you approve `widget-lib` in one project. As `enable-if-dependency`, it loads in every project that depends on `widget-lib` and nowhere else. As `enable-always`, it would load in every project, fetched from crates.io, whether or not that project uses it.

### Why not `disable-always`?

There is no conditional disable to tell it apart from, so the suffix adds nothing, and `disable` is already released.

### Why not `enable-crate` and `disable-crate`?

These lists also name registry plugins (`enable-always` wakes a dormant one, `disable` turns one off), and other package managers will add packages that are not crates.

## Implementation plan and status

### Step 1: Rename the keys

Read `use` and `auto-enable` as the new keys with a warning, write only the new ones, and update the consent prompt, `status` and the reference. Verified by config tests for old, new and mixed spellings, and a save writing only the new keys.

- [ ] Implemented

### Step 2: `enable`, `disable` and `reset`

Add the three commands and keep `use` as a hidden alias. Verified by CLI tests for each command, `reset` failing on an unknown name, and the alias.

- [ ] Implemented
