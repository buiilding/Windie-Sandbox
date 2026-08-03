# Windie Dev

This folder contains local dev clients used to inspect and exercise Windie
runtime primitives.

## Windie Inspector

`windie-inspector/` is the React browser client for the localhost Windie API.
It is not part of the runtime boundary: it must call explicit API primitives
and must not own provider logic, persistence, context construction, runtime
state transitions, tool execution, or permission policy.

Run it from this repo with:

```bash
target/release/windie inspector start
```

Start the API independently from the repository root:

```bash
target/release/windie api start
```

For a production-style local Inspector, build the frontend and run
`windie inspector start`; it serves the standalone UI on port 3000 by default.
Set `WINDIE_INSPECTOR_PORT` or `WINDIE_INSPECTOR_ADDRESS` to change the
Inspector port. The Inspector calls the localhost API without an API token.
