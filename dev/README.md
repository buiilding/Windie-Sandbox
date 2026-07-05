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
cd dev/windie-inspector
npm install --legacy-peer-deps
npm run start
```

Start the API from the repository root:

```bash
target/release/windie api
```

Open the inspector with the printed API token:

```text
http://localhost:3000?windie_token=<printed token>
```
