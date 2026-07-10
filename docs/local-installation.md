# Local Installation

From the Windie repository root, run:

```bash
./scripts/install-local.sh
```

This command runs the project checks, builds the optimized Rust binary, builds
the operator UI, installs both in a versioned directory under
`~/.local/lib/windie/releases/`, and atomically updates
`~/.local/bin/windie` to the new release.

Add the local binary directory to the current shell and configure it for future
shells:

```bash
export PATH="$HOME/.local/bin:$PATH"
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

The `.zshrc` update is a one-time setup. Verify the installation with:

```bash
windie doctor
```

Start the installed runtime with:

```bash
windie api
```

The API starts Bifrost when necessary and prints an authenticated operator UI
URL. The installed UI is served by the same Rust process on port `8787`; it
does not require a separate Node development server. Both the binary and UI
come from the most recent successful `scripts/install-local.sh` run. Editing
files in the source checkout does not change this installed release.

Editing the source checkout does not update a running Windie installation. To
activate UI or Rust changes:

1. Run `./scripts/install-local.sh` from the repository root.
2. Stop the current `windie api` process with `Ctrl-C`.
3. Run `windie api` again.

The restarted process uses the newly installed binary and UI. Windie does not
restart itself during installation, which prevents an active runtime from being
interrupted without an explicit user action.
