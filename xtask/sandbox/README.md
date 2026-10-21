# `cargo xtask sandbox`

Runs the real interactive agent CLI (today: Claude Code) against a freshly built
Symposium, inside an [`srt`](https://github.com/anthropic-experimental/sandbox-runtime)
sandbox, and checks what the agent says and logs. It sees what the SDK/ACP agent
tests cannot: the CLI's startup order, skill loading and skill watcher.

- **`srt`** is the fence: OS sandbox, read/write rules, network allowlist.
- **This tool** is the rig: a clean fake machine, agent setup, driving the CLI
  through tmux, credentials, expectations, run folders and verdicts.

## Prerequisites

- macOS (only platform tested), run from an **unsandboxed** terminal: `srt`
  cannot nest inside another macOS sandbox (`check` says so).
- `npm i -g @anthropic-ai/sandbox-runtime`
- `tmux`, the agent CLI (`claude`), and `rustup`.
- A Claude credential, from `SANDBOX_CLAUDE_TOKEN`, or the Keychain entry
  `symposium-srt-claude-token` (service name overridable with
  `SANDBOX_CLAUDE_KEYCHAIN_SERVICE`). Create a token with `claude setup-token`:
  `security add-generic-password -a "$USER" -s symposium-srt-claude-token -w <token>`.

## Commands

```
cargo xtask sandbox check
cargo xtask sandbox list
cargo xtask sandbox run <scenario> [--variant <v>]... [--label <l>] [--agent claude]
                                   [--keep] [--no-build] [--binary <path>]
```

`run` builds `cargo-agents` first (unless `--no-build`), runs each variant (all
by default), prints one verdict line per variant plus the run folder, and exits
non-zero if any failed. `--binary` (or `SYMPOSIUM_SANDBOX_BINARY`) tests another
build, e.g. a fix branch's `target/debug/cargo-agents`; the file must be named
`cargo-agents`.

Run folders: `/tmp/symposium-sandbox/runs/<scenario>-<variant>-<label>/`, kept
after every run (`--keep` also keeps the short TMPDIR). Each holds
`summary.json` (versions, per-expectation results, verdict), the pane captures,
`agent-debug.log`, `launch.sh` (no secret), `srt.json`, and `home/.symposium/logs`.

## Scenarios

`xtask/sandbox/scenarios/<name>.toml`:

```toml
description = "..."
fixtures = ["some-fixture"]     # tests/fixtures/<name>, overlaid in order
network = ["example.com"]       # extra domains (api.anthropic.com is implied)

[symposium]
agent = "claude"
hook_scope = "global"
config = '''...'''              # written to $HOME/.symposium/config.toml before init

[timeouts]
startup = 60                    # wait for the input box
answer = 90                     # wait for an answer (polls until the agent is idle)

[[files]]                       # inline files; "~/" prefix = under HOME
path = "Cargo.toml"
content = '''...'''

[[variants]]
name = "fresh"
dirs = [".claude/skills"]       # extra setup per variant; `files` works too

[[steps]]                       # one action key per step, run in order
wait = 30                       # | prompt = "..."
                                # | expect_answer = "regex"  (answer lines of the last prompt)
                                # | expect_log = "regex"  / reject_log = "regex"  (+ count = N for exactly N)
                                # | expect_file = "path"
```

Fixture overlay mirrors `symposium_testlib`: text files (`toml md json txt ts js sh`)
expand `$TEST_DIR` (the project), `$BINARY` and `$HOME`; a fixture's `dot-symposium/`
becomes `$HOME/.symposium/`.

## Rules the rig follows (each cost debugging time)

1. Fake HOME, not `CLAUDE_CONFIG_DIR`; `XDG_*`, `SYMPOSIUM_HOME` unset.
2. Run folder outside any cargo workspace, canonical short path under `/tmp`.
3. Short `TMPDIR`: macOS caps Unix socket paths at 104 bytes (`srt` makes sockets).
4. `allowPty: true`, or Claude cannot enter raw mode.
5. Bare `.claude/` pre-created: `srt` blocks `mkdir .claude` inside the sandbox.
6. Private tmux server (`-L symposium-sandbox`); the user's own is never touched.
7. `env -i` with an explicit allowlist in `launch.sh`.
8. The token reaches tmux only via `-e`; never on disk, in argv or in logs.
9. Absolute tool paths (no asdf shims under a fake HOME).
10. `srt` policy: write only the run folder and TMPDIR, deny reading the real
    agent/Symposium/ssh/aws/config dirs, network allowlist.
11. Private `CARGO_HOME`, `CARGO_NET_OFFLINE=true` (fixtures use path deps).
12. Never Ctrl-C (exits Claude Code); inputs are cleared with Backspace.
13. Only its own run folders are ever deleted.

## Why not CI

It needs a real login and an interactive terminal driven through tmux, and the
agent's behaviour changes between versions (the Claude version is recorded in
each summary).
