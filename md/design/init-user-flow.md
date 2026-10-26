# `cargo agents init`

Sets up the user-wide configuration.

## Flow

1. **Prompt for agents** — ask which agents the user uses (e.g., Antigravity CLI, Claude Code, Copilot). Multiple agents can be selected.

2. **Write user config** — create `~/.symposium/config.toml` with the `[[agent]]` entries populated:

   ```toml
   [[agent]]
   name = "claude"

   [[agent]]
   name = "antigravity"
   ```

3. **Register hooks**: register global hooks and MCP servers for each selected agent. Also unregisters hooks for any agents that were removed, and removes the MCP servers symposium registered for them in their user-level file (see the [MCP ledger](./state.md#mcp-server-ledger)).

If `--add-agent` or `--remove-agent` flags are provided, the interactive prompt is skipped and the specified changes are applied to the existing agent list.

An `[[agent]]` name this version does not know is kept as is until `--remove-agent` removes it, and hook registration skips it with a warning, as `sync` does. Only `--add-agent` names and `--remove-agent` names that are not configured must be known agents.

Without a terminal and without those flags, `init` keeps the existing agent list, or configures Claude Code (`Agent::Claude`) when there is none. The default is named rather than taken from `Agent::all()`, whose order only drives the prompt.
