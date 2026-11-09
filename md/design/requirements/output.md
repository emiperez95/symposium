# Output requirements

`Output` carries Symposium's human-facing status messages. These requirements say where those messages go in each mode. Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} quiet
A quiet `Output` MUST NOT print any message.
:::

*Source:* Documented in src/output.rs:3-4 (module doc) "All user-visible status messages go through `Output`, which can be silenced with `--quiet` or when running from a hook.".

:::{req} capture
A capturing `Output` MUST record every message, warnings included, in order, instead of printing it.
:::

*Source:* Documented in src/output.rs:31-32 (doc comment) "Create an output that captures all messages into a buffer instead of printing them.".

:::{req} warnings_stderr
A normal `Output` MUST print warnings to stderr.
:::

*Source:* Inferred from the code only (`Output::warn`, `Output::emit_err`); needs maintainer review.

:::{req} messages_stdout
A normal `Output` MUST print every message other than a warning to stdout.
:::

*Source:* Inferred from the code only (`Output::emit` and the message helpers); needs maintainer review.

:::{req} display_path
Symposium SHOULD display a path under the user's home directory with the home prefix replaced by `~`.
:::

*Source:* Documented in src/output.rs:133 (doc comment) "Replace the home directory prefix with `~` for display.".
