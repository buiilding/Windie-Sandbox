# Local Installation

From a source checkout, install dependencies once and verify the checkout:

```bash
scripts/setup.sh
scripts/check.sh
```

Build and promote a local release:

```bash
scripts/install.sh
```

The install command builds the optimized Rust binary and operator UI, installs
both under `~/.local/lib/windie/releases/`, and atomically updates
`~/.local/bin/windie`. It does not rerun checks or install frontend dependencies.

Add the local binary directory to the current shell and configure it for future
shells:

```bash
export PATH="$HOME/.local/bin:$PATH"
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Verify the installed paths and integrations:

```bash
windie doctor
```

Start the installed runtime:

```bash
windie api
```

The installed UI is served by the Rust process on port `8787`; it does not need
the React development server. Editing the source checkout does not change the
installed release.

To activate later source changes:

1. Run `scripts/check.sh`.
2. Run `scripts/install.sh`.
3. Stop the existing `windie api` process with `Ctrl-C`.
4. Start `windie api` again.

Installation does not restart a running process, so it cannot silently
interrupt active work.
