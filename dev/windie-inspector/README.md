# Windie Inspector

This React application is Windie's editable developer preview and the source
for the compiled operator UI bundled into releases.

From the repository root:

```bash
scripts/setup.sh
scripts/dev.sh
```

Open `http://localhost:3000?windie_token=<printed token>`. The development page
may hot reload while code changes. Runtime actions are created as backend-owned
runs, so reload reconnects to `/api/runs/<run_id>/events` and replays events by
sequence number instead of cancelling work.

`npm run build` writes the production assets to `build/`. `windie api` serves
that directory in a source checkout, and release packaging places it beside
the installed executable as `ui/`.
