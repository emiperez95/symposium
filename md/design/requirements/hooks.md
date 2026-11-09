# Hook requirements

An agent runs `cargo agents hook` itself, with no person watching the terminal, and parses everything the process writes to stdout as its hook response. These requirements keep the hook path safe to run unattended. Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} no_prompt
A `cargo agents` process started by an agent hook MUST NOT read from stdin to ask the user a question. Symposium MUST prompt only when its output is neither quiet nor capturing and both stdin and stdout are terminals.
:::

A hook that blocked on a prompt would hang the agent's session. A bare terminal check is not enough: `cargo test` inherits the developer's terminal, so the library test harness would block too.

:::{req} stdout_protocol
A `cargo agents hook` process MUST write nothing to stdout except the agent's hook response. Progress messages and the output of plugin install commands MUST NOT reach stdout; Symposium MAY drop them, log them, or write them to stderr.
:::

Most agents discard the whole response when a single line ahead of the JSON fails to parse, so one stray progress message silently drops every plugin's context.

:::{example} pending_consent
A workspace depends on a crate that ships a plugin the user has not enabled or declined. When an agent starts a session, the `SessionStart` hook names that crate in the agent's context instead of asking, and for every agent that takes JSON its stdout parses as JSON. {cite}`design.requirements.hooks.no_prompt` {cite}`design.requirements.hooks.stdout_protocol`
:::

## Dispatch details

The requirements below were derived from `src/hook.rs` and its design docs; each says where its behavior is documented.

:::{req} unsupported_event
A `cargo agents hook` process asked for an event its agent does not support MUST exit with a failure status.
:::

*Source:* Inferred from the code only (`execute_hook` bail, `run` → `ExitCode::FAILURE`); needs maintainer review.

*Note:* the draft's "and write nothing to stdout" is already covered by `stdout_protocol`.

:::{req} non_occurrence_neutral
When an agent's payload is not an occurrence of the symposium event it maps to, Symposium MUST answer with the agent's neutral output.
:::

*Source:* Documented in md/design/hook-flow.md:7 "Such a payload is answered with the agent's neutral output".

:::{req} non_occurrence_no_dispatch
When an agent's payload is not an occurrence of the symposium event it maps to, Symposium MUST NOT run auto-sync or any plugin hook.
:::

*Source:* Documented in md/design/hook-flow.md:7 "and skips every step below".

:::{req} auto_sync_session_start
On `SessionStart`, when `auto-sync` is enabled and the hook's working directory is in a workspace, Symposium MUST sync skills before dispatching, checking upstream sources for changes.
:::

*Source:* Documented in md/design/hook-flow.md:13 "it bypasses the freshness gate ... and passes `UpdateLevel::Check`".

:::{req} auto_sync_gate
On events other than `SessionStart`, when `auto-sync` is enabled, Symposium MUST sync before dispatching exactly when an input recorded by the previous sync has changed.
:::

*Source:* Documented in md/design/hook-flow.md:13 "Every other event keeps the cheap path, gated on the package managers' recorded inputs".

:::{req} best_effort
Apart from a plugin hook that blocks, a failure while auto-syncing, writing logs, acquiring or refreshing a hook's sources, or running a single plugin hook MUST NOT stop `cargo agents hook` from answering the agent.
:::

*Source:* Documented in md/design/hook-flow.md:9 "Runs quietly and non-fatally — failures are logged but don't block hook dispatch"; md/reference/plugin-definition.md:401 "Requirement installation is best-effort: failures are logged and dispatch continues."; md/design/module-structure.md:203 "`prewarm_hook_sources` (best-effort, ...)".

:::{req} source_refresh_session_start
On `SessionStart`, when `auto-sync` is enabled, Symposium MUST check upstream for a newer version of each applicable plugin hook source that is already installed.
:::

*Source:* Documented in md/design/module-structure.md:203 "it walks every applicable plugin's hooks and *refreshes* each installation's already-cached source via `refresh_installation_if_present` (`UpdateLevel::Check`)".

:::{req} source_refresh_installed_only
The `SessionStart` source refresh MUST NOT install a plugin hook source that was never installed.
:::

*Source:* Documented in md/design/module-structure.md:203 "It is **refresh-only**: a source that was never acquired is left alone".

:::{req} dispatch_serves_cache
Dispatching a plugin hook MUST use the installed version of each cargo source it needs rather than querying for a newer one.
:::

*Source:* Documented in md/design/module-structure.md:96 "`None` reads the pointer and serves that version with **no crates.io query** — so a per-event dispatch never hits the registry".

*Note:* deliberately narrowed to cargo sources. Under `None`, GitHub sources still run debounced freshness checks (module-structure.md:98), so "no upstream check at dispatch" would be false for them.

:::{req} discovery_hint
On `SessionStart`, when the workspace exposes applicable plugin subcommands, Symposium MUST add context telling the agent to run `cargo agents --help`.
:::

*Source:* Documented in md/design/hook-flow.md:16 "a line suggesting the agent run `cargo agents --help` to find them"; md/design/subcommands.md:114.

:::{req} discovery_hint_audience
The discovery hint MUST tell the agent to use only the commands listed for agents unless the user asks otherwise.
:::

*Source:* Inferred from the code only (`discovery_hint`); needs maintainer review.

:::{req} consent_hint_names
On `SessionStart`, when workspace dependencies ship plugins the user has neither enabled nor declined, Symposium MUST add context naming each of those dependencies once.
:::

*Source:* Documented in md/design/hook-flow.md:17 "**Consent hint**: the dependency plugins awaiting consent"; md/reference/cargo-agents-sync.md:47 "pending candidates are named when a session starts".

:::{req} consent_hint_defers
The consent hint MUST tell the agent to point the user at `cargo agents sync` or `cargo agents use` rather than enable the plugins itself.
:::

*Source:* Documented in md/design/hook-flow.md:17 "telling the agent to point the user at `cargo agents sync` or `cargo agents use`".

:::{req} update_nudge
On `SessionStart`, when `auto-update` is `warn`, the update-check throttle has elapsed, and a newer Symposium version is published, Symposium SHOULD add context suggesting `cargo agents self-update`.
:::

*Source:* Documented in md/design/hook-flow.md:18 "when `auto-update = \"warn\"`, the 24-hour check throttle has elapsed, and the registry reports a newer version"; md/reference/cargo-agents-self-update.md:26.

:::{req} new_plugins_notice
On `PostToolUse` and `UserPromptSubmit` right after auto-sync re-synced because a package-manager input changed, Symposium MUST add context telling the agent to point the user at `cargo agents sync`.
:::

*Source:* Documented in md/design/hook-flow.md:20 "a generic notice says there may be new plugins and points at `cargo agents sync`".

:::{req} builtin_quiet
Except for the new-plugins notice, Symposium's built-in handling MUST add no context on events other than `SessionStart`.
:::

*Source:* Documented in md/design/hook-flow.md:15 "`PreToolUse` produces nothing."; :20 "`PreToolUse` or `Stop`, which sync but carry no notice".

:::{req} hook_selection_one
For each plugin, Symposium MUST deliver an event to at most one of its hooks.
:::

*Source:* Documented in md/design/hooks.md:14 "For each plugin, it picks at most one hook to deliver".

*Note:* if a plugin has two matching hooks of the same format, the last one wins. That is incidental.

:::{req} hook_selection_native_first
Symposium MUST select a plugin's hook in the host agent's format over its symposium-format hook.
:::

*Source:* Documented in md/design/hooks.md:16-17 "If the plugin declares a hook with `format` matching **agent A** → deliver ... Otherwise, if the plugin declares a **symposium-format** hook".

:::{req} foreign_format_silent
A plugin hook in another agent's format MUST NOT fire.
:::

*Source:* Documented in md/design/hooks.md:20 "A `format = \"claude\"` hook will only fire on Claude — it won't be translated for Copilot or Antigravity.".

:::{req} hook_gating
A plugin hook MUST fire only when its event and matcher match the payload, its plugin's predicates hold, and its own predicates hold.
:::

*Source:* Documented in md/reference/plugin-definition.md:248 (`matcher`), :256 "Predicates ... that must all hold for the hook to dispatch"; md/design/module-structure.md:143 "hook dispatch evaluates the plugin-level set ... plus the hook-level set".

:::{req} native_input
Symposium MUST give a plugin hook in the host agent's format the agent's own payload, unchanged.
:::

*Source:* Documented in md/design/hooks.md:16 "deliver the input unmodified"; md/reference/plugin-definition.md:255 "receives that agent's native input unchanged".

:::{req} native_output
Symposium MUST read the output of a plugin hook in the host agent's format as that agent's output.
:::

*Source:* Documented in md/design/hooks.md:40 "Native format matching the host agent → pass through directly.".

:::{req} canonical_input
Symposium MUST give a symposium-format plugin hook the event in symposium canonical form.
:::

*Source:* Documented in md/design/hooks.md:17 "convert to symposium canonical and deliver".

:::{req} canonical_output
Symposium MUST translate a symposium-format plugin hook's output to the host agent's wire format.
:::

*Source:* Documented in md/design/hooks.md:41 "Symposium format → convert to host agent's wire format."; md/reference/hook-events.md:3 "Symposium converts to and from each agent's native wire format".

:::{req} hook_spawn_runnable
Symposium MUST run a plugin hook's executable directly, or its script through `sh`, passing the hook's arguments.
:::

*Source:* Documented in md/reference/plugin-definition.md:148 "invoked as `sh <path> <args>`"; md/design/hook-flow.md:32 "Spawn `path args…` directly for `Exec`, or `sh path args…` for `Script`.".

:::{req} hook_stdin_payload
Symposium MUST write the event payload to a plugin hook's stdin.
:::

*Source:* Documented in md/design/hook-flow.md:33 "Pass the event JSON (in the selected format) on stdin to the plugin's hook.".

:::{req} hook_output_captured
Symposium MUST capture a plugin hook's stdout and stderr rather than let the hook inherit its own.
:::

*Source:* Documented in md/reference/plugin-definition.md:440 "Otherwise stderr is captured but not returned on success."; :434 "the hook's stdout is parsed as JSON".

:::{req} hook_env_path_var
Symposium MUST run a plugin hook with `SYMPOSIUM_<name>` set to the absolute path of each of its installations that resolves to a runnable with an absolute path, where `<name>` is the installation name with every character that is not an ASCII letter or digit replaced by `_`.
:::

*Source:* Documented in md/reference/plugin-definition.md:410 "`$SYMPOSIUM_<name>` | Installation resolves to a runnable with an absolute path"; :413 (name rule).

:::{req} hook_env_dir_var
Symposium MUST run a plugin hook with `SYMPOSIUM_DIR_<name>` set to the directory of each of its installations that has a Symposium-managed cache, with `<name>` formed as for `SYMPOSIUM_<name>`.
:::

*Source:* Documented in md/reference/plugin-definition.md:409 "`$SYMPOSIUM_DIR_<name>` | Installation has a symposium-managed cache".

:::{req} hook_env_path_prepend
Symposium MUST prepend to a plugin hook's `PATH` the parent directory of each installation runnable with an absolute path, with the hook command's own directory first.
:::

*Source:* Documented in md/reference/plugin-definition.md:411 "Each runnable's parent dir is prepended, with the hook's `command` first.".

:::{req} merge_later_wins
When Symposium combines the built-in response and each plugin hook's response, a field from a later response MUST replace the same field from an earlier one.
:::

*Source:* Documented in md/reference/plugin-definition.md:438 "Valid JSON objects are merged together across successful hooks; keys from later hooks overwrite earlier keys.".

:::{req} merge_null_removes
When Symposium combines responses, a `null` field in a later response MUST remove that field.
:::

*Source:* Documented in src/hook.rs:955 (doc comment on `merge`) "Fields with null values in `b` will delete the corresponding field in `a`.".

*Note:* whether any hook relies on this is unknown. Possibly incidental. It is a candidate to downgrade to `internal_detail` if maintainers say so.

:::{req} joined_context
On `SessionStart`, `UserPromptSubmit` and `PostToolUse`, the response MUST carry the context of every source that supplied one, the built-in context first and then each plugin hook's in dispatch order.
:::

*Source:* Documented in md/reference/plugin-definition.md:438 "symposium's own context and every hook's are joined, in order, so none replaces another"; md/design/hook-flow.md:36.

:::{req} deny_final
When a plugin hook's output denies a tool call, Symposium MUST NOT run any later plugin hook for that event.
:::

*Source:* Documented in src/hook.rs:945-946 (doc comment) "A denial is final, like exit 2"; tests/hook_decisions.rs:88-89 "the denial stays final".

:::{req} exit_2_stops
When a plugin hook exits with status 2 or reports no exit status, Symposium MUST block and run no later plugin hook for that event.
:::

*Source:* Documented in md/reference/plugin-definition.md:435 "`2` (or no reported exit code): block. The action is blocked, dispatch stops".

:::{req} block_reason_stderr
When a plugin hook blocks by its exit status, Symposium MUST give the agent the hook's stderr as the reason.
:::

*Source:* Documented in md/reference/plugin-definition.md:435 "the hook's stderr becomes the reason the agent is given".

:::{req} block_pre_tool_use_deny
On `PreToolUse`, Symposium MUST deliver a block by exit status as a denial of the tool call in the host agent's own format.
:::

*Source:* Documented in md/reference/hook-events.md:232 "On `PreToolUse` this is a `\"deny\"` decision, delivered to each agent as in the table above.".

:::{req} block_other_exit_2
On events other than `PreToolUse`, Symposium MUST deliver a block by exit status by exiting 2 with the reason on stderr.
:::

*Source:* Documented in md/reference/hook-events.md:232 "On other events `cargo agents hook` exits 2 with the reason on stderr".

:::{req} hook_trace
When `SYMPOSIUM_HOOK_TRACE` names a file, `cargo agents hook` MUST append one JSON line per answered event recording the agent, the event, the input, the output, the exit code and stderr.
:::

*Source:* Documented in src/hook.rs:372-373 (doc comment) "If `SYMPOSIUM_HOOK_TRACE` is set to a file path, append a JSONL entry. Used for integration testing".

*Note:* this is a test-infrastructure interface. The field list itself is inferred from code. A failed `execute_hook` writes no line.

:::{req} raw_json_fallback
Symposium MAY merge the output of a symposium-format plugin hook that is a JSON object but not a canonical output event into the response unchanged.
:::

*Source:* Documented in md/reference/plugin-definition.md:438 "Valid JSON objects are merged together across successful hooks".

*Note:* Open question for maintainers: `md/design/hooks.md` and `md/reference/hook-events.md` say symposium-format output is converted to the host agent's format, and this fallback skips the deny-is-final check and context joining. Kept as current behavior until that is decided.
