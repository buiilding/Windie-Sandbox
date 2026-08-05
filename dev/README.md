# Windie Dev

This folder contains development documentation and local helper context. The
first-party Inspector component is maintained separately in the pinned
`vendor/windie-inspector/` submodule.

## Windie Inspector

`vendor/windie-inspector/frontend/` is the React browser client for the localhost Windie API, and
`vendor/windie-inspector/host/` is its optional static-asset host. Neither is part of the
runtime boundary: the client must call explicit API primitives
and must not own provider logic, persistence, context construction, runtime
state transitions, tool execution, or permission policy.

The repository development supervisor runs the API and Inspector in the
foreground, keeps their output attached to the terminal, and stops them with
Ctrl-C:

```bash
source ./activate_windie-dev
windie-dev dev up
```

Run one component when debugging a single boundary:

```bash
windie-dev dev run api
windie-dev dev run inspector
windie-dev dev run gateway
```

`inspector` uses `npm start` in `vendor/windie-inspector/frontend` and hot reloads browser changes. Rust API changes
hot reload when `cargo-watch` is installed; otherwise the API runs once and
must be restarted. The gateway runs Bifrost through Air, rebuilding and
restarting on Go changes. Install the two optional watchers when needed:

```bash
cargo install cargo-watch
go install github.com/air-verse/air@latest
```

Check or stop the development stack with:

```bash
windie-dev dev status
windie-dev dev down
```

For a local release install, use the same repository binary:

```bash
windie-dev release build
windie-dev release install
windie-dev release verify
source ./activate_windie
windie status
```

Set `WINDIE_GATEWAY_PORT`, `WINDIE_API_PORT`, and `WINDIE_INSPECTOR_PORT` before
starting either workflow when two local installations need to run together.
The installed release Inspector is embedded and does not hot reload; use
`windie-dev dev up` for frontend development.
