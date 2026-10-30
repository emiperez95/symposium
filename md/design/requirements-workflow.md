# Requirements and evidence

Some behavior in Symposium is stated as a named requirement with checked evidence, using [Habeas Codicem](https://nikomatsakis.github.io/habeas-codicem/). A requirement is a `:::{req}` block in a chapter under `md/design/requirements/`; its evidence is a JSON file under `evidence/` recording the source it relies on and why that source meets it. `cargo test` fails when the evidence is missing, judged false, or out of date with the code it quotes. The [requirements index](./requirements/index.md) shows the state of each one.

Adoption is incremental. Only the areas with a requirements chapter are covered, starting with the [hook requirements](./requirements/hooks.md).

## Setup

Install the CLI once. The book build needs it too, since `book.toml` runs `habeas mdbook` as a preprocessor:

```bash
cargo install --locked habeas@0.2.0
```

Keep the CLI at the version `Cargo.toml` pins. Don't re-run `habeas setup`: this repo has replaced the `tests/habeas.rs` and trimmed the `md/habeas.css` it generates, so setup refuses to continue. Habeas's `SessionStart` hook only checks `book.toml`, which is already configured.

## Names and references

A definition's full name is its chapter path plus its own name: `no_prompt` in `md/design/requirements/hooks.md` is `design.requirements.hooks.no_prompt`. Requirements use MUST/SHOULD wording. An `:::{example}` describes one concrete case (situation, action, result) and cites the requirement it illustrates.

Code points back at a definition with a comment:

```rust
// {impl}`design.requirements.hooks.no_prompt`   -- this line implements it
// {test}`design.requirements.hooks.no_prompt`   -- this test checks it
```

An example judged true needs at least one `{test}` reference.

## Recording evidence

Capture inputs with `habeas add-input` rather than writing JSON by hand:

```bash
habeas add-input --for-requirement design.requirements.hooks.no_prompt --rust-item src/output.rs:61
habeas add-input --for-requirement design.requirements.hooks.no_prompt --file-range src/init.rs:83-83
habeas add-input --for-example design.requirements.hooks.pending_consent --requirements 'design.requirements.hooks.*'
```

The first `add-input` creates the evidence file with `determination: false`. Every `{impl}`/`{test}`/`{cite}` line must fall inside one of its inputs. Once the inputs cover the behavior, read them, run the tests they cite, and write a `rationale` saying why the requirement holds before setting `determination` to `true`.

## When code changes

Inputs store the exact lines they quote, so editing quoted code makes `cargo test` fail with a diff until the evidence is refreshed:

- `--rust-item` inputs follow the item when it moves, and go stale only when its text changes. Prefer them.
- `--file-range` inputs are pinned to line numbers, so inserting a line anywhere above one makes it stale, even in unrelated code. Avoid them. When the lines you need sit inside a large function, move them into a small named helper and quote that, as `command_output` and `prints_reports` in `src/bin/cargo-agents.rs` do. Quoting a large function instead makes every edit to it stale the evidence.

To refresh, re-run `add-input` (dropping the stale input from the JSON first if its range moved), reread the requirement against the new code, and set `determination` back to `true` only if it still holds. `habeas status 'design.*'` lists what needs attention; `habeas info <name>` prints one definition with its evidence.

## The per-function rule

Upstream Habeas also requires every Rust function in the repository to cite a requirement, with a per-file attestation under `evidence/rust/`. That is several thousand functions here, so `tests/habeas.rs` enforces those two rules only for the files listed in its `FUNCTION_RULE_FILES` (none yet); every other check applies everywhere. Two consequences:

- `habeas status` and `habeas info` still print the uncited-function findings for every file. Ignore them outside the allowlist.
- Habeas's agent skills (offered by `cargo agents sync` once the `habeas` dev-dependency's plugin is enabled) assume the full rule. In this repo, only cite every function in a file you are adding to `FUNCTION_RULE_FILES`.

The filter in `tests/habeas.rs` matches Habeas's error messages, which is why `habeas` is pinned to an exact version. Two checks guard it across upgrades: the `habeas` test fails if no error matches the filter at all, and `function_rule_file_parses_habeas_errors` pins the exact error shapes (including Windows `\` paths). When upgrading, update those sample errors from the new version's source.
