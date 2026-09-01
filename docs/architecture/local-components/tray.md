# Tray

## Purpose

<!-- Explain the desktop status and explicit component-control surface. -->

## Owns

<!-- Describe health polling, menus, and user-requested API or gateway controls. -->

## Does not own

<!-- Distinguish the tray from runtime supervision and notifications. -->

## Main flow

1. <!-- Poll the relevant local component health endpoints. -->
2. <!-- Present current status and available controls. -->
3. <!-- Forward an explicit start or stop request. -->

## Important invariants

- <!-- The tray does not become the parent supervisor of local components. -->
- <!-- Closing the tray does not stop API-owned session work. -->

## Related code

- <!-- `src/local/tray.rs` -->
- <!-- `src/operation/system.rs` -->
