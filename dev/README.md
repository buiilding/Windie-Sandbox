# Windie Dev

This folder contains local dev clients used to inspect and exercise Windie
runtime primitives.

## Windie Inspector

`windie-inspector/` is the React browser client for the localhost Windie API.
It is not part of the runtime boundary: it must call explicit API primitives
and must not own provider logic, persistence, context construction, runtime
state transitions, tool execution, or permission policy.

Run the hot-reloading preview from this repo with:

```bash
scripts/dev-ui.sh
```

Start an isolated development API from the repository root:

```bash
scripts/dev-api.sh
```

Open the inspector with the printed API token:

```text
http://localhost:3000?windie_token=<printed token>
```

For self-editing work, keep the active coding conversation in the operator UI
served by an installed `windie api`. Use this port-3000 client only as the
editable preview. Hot reload disconnects its event subscription, then replays
the active backend run; it does not own or cancel the loop.
