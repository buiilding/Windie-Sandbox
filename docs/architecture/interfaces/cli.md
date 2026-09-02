# CLI

The CLI is Windie's terminal interface. It reads command-line arguments,
converts them into typed commands, and sends those commands to the shared
runtime and persistence operations.

The concrete command, option, and output reference is in
[`docs/index/CLI.md`](../../index/CLI.md). This document explains what the CLI
owns and how it fits with the other Windie interfaces.

## Purpose

The CLI lets a user operate Windie from a terminal without going through the
browser Inspector. It supports local runtime lifecycle commands, conversation
and message operations, tool and session operations, onboarding, and
repository development workflows.

## Owns

The CLI owns the terminal-facing boundary:

- parsing `argv` into the typed `Command` contract;
- validating command shapes, identifiers, options, and flags;
- dispatching parsed commands to the appropriate terminal adapter;
- presenting human-readable output, stable JSON reports, help, and version
  information; and
- returning conventional invalid-usage behavior when a command cannot be
  parsed.

The CLI has adapters for system and process lifecycle commands, conversations
and messages, tools, durable sessions, environment values, onboarding, and
development, release, marketplace, and benchmark workflows.

## Does not own

The CLI does not define a second conversation model or runtime policy. Its
adapters call the shared operation layer and use the same SQLite store,
conversation tree, session state, tool policy, context compilation, and Bifrost
boundary used by the API.

The CLI does not own browser presentation, HTTP routing, provider inference, or
the durable data model. Terminal formatting reports the result of an
operation; it does not make runtime decisions.

## Main flow

1. `main` calls `cli::read`, which passes the process arguments to the CLI
   parser.
2. The parser matches the argument shape and returns a typed `Command`. An
   unsupported shape becomes `Command::Invalid` and the CLI prints usage with
   exit code `2`.
3. `cli::adapter::run` dispatches the typed command to a domain-specific
   adapter. Development, release, marketplace, and benchmark commands are
   dispatched to their workflow adapters.
4. The adapter invokes the relevant shared operation, store method, or local
   process boundary. For `windie run start`, the CLI claims session execution
   with the CLI owner and advances the same runtime loop used by API sessions.
5. The adapter sends the result to `TerminalOutput`, which prints terminal
   lines or a machine-readable JSON report. Session execution also records the
   same durable session events that API-owned execution exposes.

## Important invariants

- CLI parsing only converts terminal arguments into typed commands; it does
  not open the database, call Bifrost, or decide runtime behavior.
- CLI and API use the same shared operations and persistence rules. Different
  presentation surfaces must not create different conversation or session
  semantics.
- Terminal output formatting does not make runtime decisions. JSON output is a
  representation of operation results, not a separate control path.
- A CLI-owned session has an explicit CLI execution claim, so session work is
  not silently treated as API-owned work.
- The command reference in [`docs/index/CLI.md`](../../index/CLI.md) should
  remain consistent with the parser and adapters.

## Related code

- [`src/main.rs`](../../../src/main.rs) — reads the command and invokes the
  CLI boundary.
- [`src/cli/command.rs`](../../../src/cli/command.rs) — typed command and
  command-group contracts.
- [`src/cli/parser.rs`](../../../src/cli/parser.rs) — converts raw `argv`
  tokens into typed commands.
- [`src/cli/adapter/`](../../../src/cli/adapter/) — dispatches commands to
  terminal-facing adapters.
- [`src/operation/`](../../../src/operation/) — shared workflows used by CLI
  and API clients.
- [`src/operation/session_cli.rs`](../../../src/operation/session_cli.rs) —
  runs session workflows with the CLI execution owner.
- [`src/output/terminal.rs`](../../../src/output/terminal.rs) — terminal
  presentation and stable help/report output.
