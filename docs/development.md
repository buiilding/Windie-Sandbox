# Development Mode

Development mode runs the current Rust checkout and hot-reloading inspector
with state isolated from the installed Windie release.

## First Setup

Install the pinned frontend dependencies once:

```bash
scripts/setup.sh
```

The setup command uses `npm ci` and does not start services or install Git
hooks.

## Start Development

```bash
scripts/dev.sh
```

One command starts:

- the current Rust checkout through `cargo run -- api` on port `8787`;
- the Vite development server for the React inspector on port `3000`;
- one stable development API token shared by both processes.

The script prints the authenticated inspector URL. It does not open a browser
or install dependencies implicitly.

Development state defaults to:

```text
target/windie-dev-data
target/windie-dev-config
target/windie-dev-api-token
```

The installed Windie database and bundled UI are not modified.

Frontend source changes hot reload. Rust source changes require stopping and
restarting `scripts/dev.sh`. Stopping the script stops both child processes.
Restarting the API interrupts active backend runs; a frontend hot reload only
disconnects and replays their event subscriptions.

To let the development API launch Bifrost with the normal provider secrets,
select the provider file explicitly:

```bash
WINDIE_ENV_FILE="$HOME/.config/windie/providers.env" scripts/dev.sh
```

## Check the Checkout

```bash
scripts/check.sh
```

The check runs:

1. `cargo fmt --check`;
2. `cargo test`;
3. `cargo clippy --all-targets -- -D warnings`;
4. the production frontend build.

It does not build benchmark baselines, call Bifrost, or send provider requests.

## Benchmarks

Benchmarks are explicit and separate from correctness checks:

```bash
scripts/bench.sh runtime
scripts/bench.sh conversation <conversation_id>
```

See `benches/README.md` for fixtures and comparison details.

## Promote a Local Release

After checks pass:

```bash
scripts/install.sh
```

The install command builds and promotes the release binary and operator UI. It
does not rerun `scripts/check.sh`.
