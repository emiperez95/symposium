# Hook requirements

An agent runs `cargo agents hook` itself, with no person watching the terminal, and parses everything the process writes to stdout as its hook response. These requirements keep the hook path safe to run unattended. Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} no_prompt
A `cargo agents` process started by an agent hook MUST NOT read from stdin to ask the user a question. Symposium MUST prompt only when its output is neither quiet nor capturing and both stdin and stdout are terminals.
:::

A hook that blocked on a prompt would hang the agent's session. A bare terminal check is not enough: `cargo test` inherits the developer's terminal, so the library test harness would block too.

:::{req} stdout_protocol
A `cargo agents hook` process MUST write nothing to stdout except the agent's hook response. Progress messages and the output of plugin install commands MUST go to stderr instead.
:::

Most agents discard the whole response when a single line ahead of the JSON fails to parse, so one stray progress message silently drops every plugin's context.

:::{example} pending_consent
A workspace depends on a crate that ships a plugin the user has not enabled or declined. When an agent starts a session, the `SessionStart` hook names that crate in the agent's context instead of asking, and for every agent that takes JSON its stdout parses as JSON. {cite}`design.requirements.hooks.no_prompt` {cite}`design.requirements.hooks.stdout_protocol`
:::
