---
name: cua-driver
description: Use the approved Windie CUA Driver for deliberate local computer interaction. Read this skill before using computer-control tools.
---

# Windie CUA Driver

Use the CUA Driver only for the specific user-requested interaction.

1. Read the current task and identify the exact visible application or page.
2. Read the platform-specific reference when the task depends on the host operating system.
3. Inspect the current state before taking an action.
4. Prefer the smallest reversible action that advances the task.
5. Do not submit forms, send messages, purchase anything, delete data, or change account settings without explicit user authorization.
6. After an action, inspect the result and report failures instead of guessing.

Supporting references:

- `MACOS.md` for macOS permissions and application behavior.
- `WINDOWS.md` for Windows permissions and application behavior.
- `LINUX.md` for Linux desktop requirements.
- `BROWSER.md` for browser-specific interaction guidance.
- `RECORDING.md` for screenshots and visual verification.
- `EMBEDDING.md` for integrating the driver into another local runtime.

The Windie runtime controls which computer-control tools are available and
which calls require approval.
