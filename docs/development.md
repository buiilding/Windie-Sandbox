# Development Mode

Development mode runs the current checkout with isolated Windie data and a
hot-reloading UI.

## API and UI Processes

Both API commands listen on port `8787`, but they run different Rust builds:

| Command | Rust runtime | UI served on port `8787` |
| --- | --- | --- |
| `windie api` | Installed release binary | UI bundled by the last local installation |
| `scripts/dev-api.sh` | Current source checkout through `cargo run` | Existing compiled development UI, when available |

The UI served on port `8787` is static and does not automatically reload when
frontend source files change. `scripts/dev-api.sh` lets developers run current
Rust source without first promoting it through `scripts/install-local.sh`.

`scripts/dev-ui.sh` starts a separate React development server at
`http://localhost:3000`. This server compiles the current frontend source and
automatically reloads UI changes. It talks to the Windie API on port `8787` and
does not replace the API process.

Start the API in the first terminal:

```bash
scripts/dev-api.sh
```

The script uses `cargo run`, which incrementally builds and runs the current
Rust source in debug mode. It stores development state under `target/` instead
of modifying the installed Windie database.

Start the UI in a second terminal:

```bash
scripts/dev-ui.sh
```

The UI runs on port `3000`, talks to the API on port `8787`, and automatically
reloads frontend source changes. The API script stores its generated
development token under `target/`, and the UI script uses that token to print
the correct authenticated port-3000 URL. After the frontend finishes compiling,
the script opens that authenticated URL in the default browser. React's bare,
unauthenticated URL is suppressed.

A UI reload disconnects and reconnects the browser client, but the active agent
run remains owned by the API process and continues running. Restarting the API
process itself interrupts an active run.

The development API uses an isolated configuration directory. To let it start
Bifrost with the normal provider secrets, select the provider file explicitly:

```bash
WINDIE_ENV_FILE="$HOME/.config/windie/providers.env" scripts/dev-api.sh
```

Rust source changes require restarting `scripts/dev-api.sh`. Frontend changes
do not require an API restart. Once development changes are tested, promote the
checkout to the installed runtime with:

```bash
./scripts/install-local.sh
```
