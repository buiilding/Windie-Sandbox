# Inspector

## Purpose

<!-- Explain the hosted browser interface for a paired local Windie runtime. -->

## Owns

<!-- Describe presentation state, tree rendering, controls, and stream consumption. -->

## Does not own

<!-- Distinguish browser state from session authority, SQLite, and model execution. -->

## Main flow

1. <!-- Authenticate and pair with the local runtime. -->
2. <!-- Load authoritative snapshots and send explicit user actions. -->
3. <!-- Render replayed and live runtime updates. -->

## Important invariants

- <!-- The Inspector never infers durable session ownership from cached state. -->
- <!-- Closing the browser does not stop API-owned execution. -->

## Related code

- <!-- `vendor/windie-inspector/frontend/src/context/WindieContext.jsx` -->
- <!-- `vendor/windie-inspector/frontend/src/hooks/useSessionRuntime.js` -->
- <!-- `vendor/windie-inspector/frontend/src/lib/windieApi.js` -->
