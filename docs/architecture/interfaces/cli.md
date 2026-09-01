# CLI

## Purpose

<!-- Explain the terminal interface to Windie's shared operations. -->

## Owns

<!-- Describe parsing, command contracts, adapters, and terminal presentation. -->

## Does not own

<!-- Distinguish CLI presentation from durable state and runtime policy. -->

## Main flow

1. <!-- Parse arguments into a typed command. -->
2. <!-- Dispatch the command through the correct operation adapter. -->
3. <!-- Render structured or human-readable output. -->

## Important invariants

- <!-- CLI and API use the same underlying store and runtime rules. -->
- <!-- Output formatting does not make runtime decisions. -->

## Related code

- <!-- `src/cli/command.rs` -->
- <!-- `src/cli/parser.rs` -->
- <!-- `src/cli/adapter/` -->
- <!-- `src/output/terminal.rs` -->
