# Contributing to Windie

Windie is a local AI runtime. Contributions should preserve clear authority
boundaries, typed contracts, explicit permission decisions, and independently
understandable components.

## Start with the development guide

Set up a checkout and run the local verification workflow in the
[development guide](docs/guides/development/README.md). Read
[`Backend.md`](docs/index/Backend.md) and
[`Frontend.md`](docs/index/Frontend.md) before changing a runtime or Inspector
boundary.

## Issue and branch workflow

Every non-release pull request closes an issue. Before changing code or
documentation, confirm that an existing issue accurately describes the
problem; create one when it does not.

Start from an up-to-date local integration branch, then create a descriptive
feature branch before making changes:

```bash
git switch main
git pull --ff-only origin main
git switch -c codex/<clear-change-name>
```

Never push `main` or `windie-2` directly. Keep those local integration
branches aligned with `origin/main`; perform feature work and commits on the
feature branch.

Issue descriptions should state the problem, intended scope, acceptance
criteria, and relevant implementation boundaries. Do not describe only a
patch that already exists.

## Engineering expectations

- Prefer direct, typed Rust over loose strings, maps, and hidden side effects.
- Keep runtime authority in its owning boundary. The Inspector presents API
  decisions; it does not infer durable session ownership or runtime policy.
- Add `//!` module documentation to Rust source files and explain meaningful
  types, functions, and non-obvious invariants.
- Keep dependencies small and justified.
- Preserve unrelated root and submodule changes. When changing the Inspector,
  commit its repository first, then update the root submodule pointer.

## Verify before opening a pull request

Run the required local checks from the repository root:

```bash
scripts/check-release-notes.sh
scripts/check-docs.sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
npm ci --legacy-peer-deps --prefix vendor/windie-inspector/frontend
npm run build --prefix vendor/windie-inspector/frontend
```

Windows CI additionally runs `cargo check` and `cargo test` for
`x86_64-pc-windows-msvc`. Run those checks locally when your change could alter
platform behavior.

Every commit must add or update a meaningful `CHANGELOG.md` entry describing
the user-facing, runtime, documentation, or developer-facing effect.

## Pull requests

Before pushing, read every included commit, review the full branch diff, and
read the issue being closed. Use this pull-request description structure:

```markdown
Closes #<issue_number>

## What changed

-
-

## Why

<Explain why the change was necessary and how it addresses the issue.>
```

Use a concise title that says what changed. Release pull requests are the only
exception to the issue-closing requirement.
