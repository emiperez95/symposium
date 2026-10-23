# Agent evaluations

`cargo xtask agent-eval` runs a real Claude Code or Codex CLI session on a small Rust task inside `srt`, with or without Symposium, then grades the code the agent left behind and checks what Symposium did along the way. The approved design and its precise rules are in the [skill evaluation RFD](../rfds/skill-evaluation/README.md) and its [reference](../rfds/skill-evaluation/reference.md). This chapter records how that design maps onto the repository.

The runner calls a model, so it never runs in CI. Its model-free parts do: task validation (`--check-tasks`) and the crate's tests.

## Where things live

| Path | Holds |
| --- | --- |
| `symposium-eval/` | The engine, as a library crate (`publish = false`). |
| `xtask/` | The `cargo xtask agent-eval` entry point. It forwards its arguments to `symposium_eval::run` and holds no evaluation logic. |
| `evals/` | Tasks, plugins, and fixture crates. Data only, excluded from the Cargo workspace. |
| `results/` | Retained batches, one directory per run. Machine-local and ignored by git. |

Trial run directories are created outside the checkout. `srt` denies the whole checkout to the agent, since it holds `evals/` and the hidden tests, so a trial cannot live inside it. Only finished results are copied back to `results/`.

The RFD places the runner in `xtask`. The command is unchanged, but the code lives in its own crate so that its tests run with `cargo test -p symposium-eval` and later evaluations can build on a library rather than on a grab-bag binary.

## Modules

Each module arrives with the RFD step that needs it:

| Module | Responsibility | RFD step |
| --- | --- | --- |
| `task` | `task.toml`, the one-test-per-`checks/`-file rule, `--check-tasks` | 1 |
| `run` | `RunDir`, `RunEnv`, the generated `srt` policy, process-tree cleanup | 2 |
| `client` | `ClientRunner` and the Claude and Codex adapters | 4, 7 |
| `grade` | The grading copy and its checks | 1 |
| `expect` | Skill, hook, and sync expectations | 4 |
| `report` | `result.json`, the batch manifest, `benchmark.json` | 4, 5 |

## Code comments

Comments answer why, an invariant, or a surprising behavior of a client or tool. They never restate what the code says.

- **Module docs (`//!`).** One summary sentence, then only what the code cannot show: the mechanism, the decision behind it, and links to the RFD rule it implements. Cargo's `fingerprint` module is the model, kept short.
- **Invariants.** Write a rule the code must keep as `Invariant:` in the module doc, and back each one with a `#[test]`, as rust-analyzer does with its architecture invariants. Example: the baseline can never reach `cargo-agents`.
- **Item docs (`///`).** One or two lines of purpose, plus an `# Errors` section on fallible public functions. Skip the doc when it would only repeat the item's name.
- **Inline comments (`//`).** Proper sentences, starting with a capital and ending with a period (rust-analyzer). Detail is welcome for a non-obvious client quirk, such as Codex appending a non-empty stdin to the prompt (rustc dev guide).
- **Error and context messages.** Lowercase, with no trailing period (rust-analyzer, Rust API guidelines).
- **Markers.** `FIXME:` for a known gap that may be merged. `TODO` only inside an unfinished PR, never merged (rustc's tidy rejects it). `SAFETY:` above any `unsafe` block (salsa).
- **Tests.** The test name states the behavior. Comment only a fixture whose setup is not obvious.

## Precedents

- **rustc's compiletest** treats tests as data: each test is a directory or file the runner discovers, so adding a test needs no code change. `evals/tasks/` follows the same idea, and `--check-tasks` plays the role of compiletest's directive validation.
- **The `cargo xtask` convention** keeps repository automation as a workspace binary invoked through a Cargo alias, with no extra tooling to install. This repository already uses it for `md-orphan-check`.
- **Cargo's own CLI test** calls clap's `Command::debug_assert` in a unit test, so a malformed argument definition fails `cargo test` instead of the first person to run the command. `symposium-eval` does the same.
