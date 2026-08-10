# Frontend mental model

The frontend is the browser-based Windie Inspector at `vendor/windie-inspector/frontend/`.

It is a thin client for the Windie localhost API. The backend owns durable
conversation, session, message, tool, provider, and gateway state. The browser
owns presentation state, user interaction state, short-lived streaming
previews, and the currently selected view.

The frontend does not read SQLite, call Bifrost directly, execute tools, or
decide what the model sees. It asks the API for authoritative snapshots and
renders them.

## Application entry point

- `src/index.js`: mounts the React application into the browser document.
- `src/App.js`: creates the application shell, `WindieProvider`, browser
  routes, and the global toast surface.
- `src/pages/Windie.jsx`: composes the inspector layout: top bar, conversation
  tree sidebar, chat panel, and optional inspector overlay. It also owns the
  first-run provider onboarding check and the persisted tree-panel toggle.

There is currently one main browser route. Routing is kept at the application
boundary so additional inspector pages can be added without moving runtime
logic into components.

## Complete frontend code map

This is the frontend equivalent of the backend file-by-file mental model. The
Paths below are relative to `vendor/windie-inspector/frontend/`.

### Application and build boundary

- `package.json`: frontend package identity, React dependencies, and `start`,
  `test`, and `build` commands.
- `package-lock.json`: resolved npm dependency graph used for reproducible
  installs. It is generated dependency state, not application behavior.
- `components.json`: shadcn-style component-generation and alias settings.
- `jsconfig.json`: JavaScript module resolution and the `@/*` alias to `src/*`.
- `craco.config.js`: Create React App/Webpack customization; installs the
  source alias, watch exclusions, optional health-check plugin, dev-server
  compatibility adapter, and optional visual-editing integration.
- `tailwind.config.js`: Tailwind content paths, dark-mode strategy, Windie
  design tokens, and animation definitions.
- `postcss.config.js`: PostCSS pipeline using Tailwind CSS and Autoprefixer.
- `public/index.html`: browser document shell containing the root element and
  static document metadata.
- `src/index.js`: React root; creates the React Query client, configures query
  defaults, imports global CSS, and mounts `App` in `StrictMode`.
- `src/App.js`: application shell; installs `WindieProvider`, browser routes,
  and the global Sonner toast renderer.
- `src/App.css`: application-level layout and Windie-specific visual rules.
- `src/index.css`: global Tailwind layers, CSS variables, theme colors,
  typography, and base browser styles.

### Pages and Windie application components

- `src/pages/Windie.jsx`: main page composition; owns overlay selection,
  onboarding auto-check, and persisted tree-panel visibility.
- `src/components/windie/TopBar.jsx`: top-level controls for conversation
  selection, tree visibility, tools/approval access, theme, and gateway/model
  status.
- `src/components/windie/Sidebar.jsx`: conversation-tree panel frame and
  collapsed/expanded layout boundary.
- `src/components/windie/TreePanel.jsx`: compact visual tree; computes a
  projected tree and layout, renders edges/nodes, and opens node context menus.
- `src/components/windie/TreeNodeContextMenu.jsx`: node actions for fork,
  truncate, and remove; sends original persisted message IDs to context
  operations.
- `src/components/windie/ConversationTreeMenu.jsx`: conversation-level menu,
  including deletion of the active conversation.
- `src/components/windie/ConversationPicker.jsx`: searchable conversation
  list, conversation creation, selection, and deletion controls.
- `src/components/windie/ChatPanel.jsx`: selected-path transcript, execution
  grouping, live execution indicator, inline approval placement, streaming
  preview placement, and scroll behavior.
- `src/components/windie/MessageRow.jsx`: role-specific message rendering,
  Markdown, image assets, reasoning, tool metadata, usage, refusal and
  annotation lanes, editing, copying, and message tree actions.
- `src/components/windie/Composer.jsx`: text composer, pasted/file image
  attachments, model picker, reasoning picker, send, continue, and stop
  controls.
- `src/components/windie/SessionsChip.jsx`: session picker and status display;
  shows active branch, queue depth, approval state, and session deletion.
- `src/components/windie/InspectorPanel.jsx`: inspector overlay for context
- `src/components/windie/InspectorPanel.jsx`: inspector overlay for context
  preview, system prompt, conversation settings, tool access mode, tool
  schemas, provider installations, and LLM provider setup.
- `src/components/windie/ToolApprovalPrompt.jsx`: inline session-owned tool
  approval surface rendered immediately above the composer when approval is
  required.
- `src/components/windie/ExtensionsPanel.jsx`: executable tool-provider
  installation lifecycle controls: setup, enable, disable, repair, and
  uninstall.
- `src/components/windie/ExtensionDetailPage.jsx`: renders the provider's
  Windie-managed README in Overview and the persisted discovered tool schemas
  in Tools; it does not invent provider documentation or discover tools itself.
- `src/components/windie/LlmProvidersPanel.jsx`: LLM provider discovery,
  provider enablement, and provider-key creation through the API.
- `src/components/windie/FloatingDeleteMenu.jsx`: positioned reusable delete
  confirmation menu for conversations and sessions.

### Shared UI primitives

The files in `src/components/ui/` are reusable Radix/Tailwind presentation
wrappers. They contain no Windie runtime or persistence rules:

- `src/components/ui/accordion.jsx`: collapsible accordion sections.
- `src/components/ui/alert-dialog.jsx`: modal confirmation and destructive-action dialogs.
- `src/components/ui/alert.jsx`: inline alert surfaces.
- `src/components/ui/aspect-ratio.jsx`: constrained aspect-ratio container.
- `src/components/ui/avatar.jsx`: avatar image, fallback, and display primitives.
- `src/components/ui/badge.jsx`: compact status/category labels.
- `src/components/ui/breadcrumb.jsx`: breadcrumb navigation primitives.
- `src/components/ui/button.jsx`: button variants and button composition.
- `src/components/ui/calendar.jsx`: calendar/date-picker presentation.
- `src/components/ui/card.jsx`: card, header, content, footer, and title surfaces.
- `src/components/ui/carousel.jsx`: carousel container and navigation primitives.
- `src/components/ui/checkbox.jsx`: checkbox control.
- `src/components/ui/collapsible.jsx`: open/closed collapsible content.
- `src/components/ui/command.jsx`: command palette/search primitives.
- `src/components/ui/context-menu.jsx`: right-click menu primitives.
- `src/components/ui/dialog.jsx`: modal dialog primitives.
- `src/components/ui/drawer.jsx`: drawer/sheet-like mobile surface.
- `src/components/ui/dropdown-menu.jsx`: dropdown menu primitives.
- `src/components/ui/form.jsx`: form-field composition helpers.
- `src/components/ui/hover-card.jsx`: hover-triggered detail card.
- `src/components/ui/input-otp.jsx`: one-time-password input primitives.
- `src/components/ui/input.jsx`: styled text input.
- `src/components/ui/label.jsx`: accessible form label.
- `src/components/ui/menubar.jsx`: horizontal menu-bar primitives.
- `src/components/ui/navigation-menu.jsx`: navigation-menu primitives.
- `src/components/ui/pagination.jsx`: pagination controls.
- `src/components/ui/popover.jsx`: anchored popover surface.
- `src/components/ui/progress.jsx`: progress indicator.
- `src/components/ui/radio-group.jsx`: radio-group controls.
- `src/components/ui/resizable.jsx`: resizable panel primitives.
- `src/components/ui/scroll-area.jsx`: styled scroll container.
- `src/components/ui/select.jsx`: select/dropdown control primitives.
- `src/components/ui/separator.jsx`: visual or semantic separator.
- `src/components/ui/sheet.jsx`: side-sheet dialog surface.
- `src/components/ui/skeleton.jsx`: loading placeholder.
- `src/components/ui/slider.jsx`: range slider control.
- `src/components/ui/sonner.jsx`: Sonner toast integration primitives.
- `src/components/ui/switch.jsx`: boolean switch control.
- `src/components/ui/table.jsx`: table, row, header, and cell primitives.
- `src/components/ui/tabs.jsx`: tab list, trigger, and content primitives.
- `src/components/ui/textarea.jsx`: styled multiline text input.
- `src/components/ui/toast.jsx`: toast structure and lifecycle primitives.
- `src/components/ui/toaster.jsx`: toast collection renderer.
- `src/components/ui/toggle-group.jsx`: grouped toggle controls.
- `src/components/ui/toggle.jsx`: single toggle control.
- `src/components/ui/tooltip.jsx`: hover/focus tooltip primitives.

### Shared application state and hooks

- `src/context/WindieContext.jsx`: shared frontend state and workflow boundary;
  loads API snapshots, exposes mutations, derives the active path/token meter,
  and composes `useSessionRuntime`.
- `src/hooks/useSessionRuntime.js`: session lifecycle adapter; asks the backend
  to resolve/create conversation-head branches, selects returned sessions,
  sends/continues/stops queries, subscribes to SSE, reduces live events,
  handles cursors, commits saved messages, and handles approvals.
- `src/hooks/useSessionRuntime.test.js`: tests session-head projection helpers
  used alongside backend-owned session resolution.
- `src/hooks/use-toast.js`: standalone reducer/store for the older reusable
  toast hook; keeps toast state in memory and schedules delayed removal.

### API, event, and data-shape libraries

- `src/lib/windieApi.js`: localhost HTTP boundary for Windie API requests;
  covers health/status, conversations, images, models, model parameters,
  sessions, approvals, providers, and conversation settings. Gateway process
  lifecycle remains a CLI concern.
- `src/lib/sessionStream.js`: localhost SSE transport; reads streamed
  session events, parses `id`, `event`, and multiline `data` fields, and turns
  failed events into client errors.
- `src/lib/sessionEventCursor.js`: event-ID ordering and duplicate suppression
  helper used during replay and live streaming.
- `src/lib/sessionEventCursor.test.js`: tests acceptance of new event IDs,
  rejection of duplicate/stale/invalid IDs, and ID-less event handling.
- `src/lib/sessionTarget.js`: contains presentation helpers for reading the
  currently selected session head; it does not decide session ownership.
- `src/lib/windieMappers.js`: maps API summaries, inspection reports, messages,
  assistant metadata, sessions, tools, providers, and installations into
  frontend shapes.
- `src/lib/treeProjection.js`: converts persisted message trees into a visual
  tree by grouping assistant tool-call/tool-result subtrees into synthetic
  expandable execution nodes.
- `src/lib/treeLayout.js`: computes balanced top-down coordinates, row sizes,
  and parent-child edges for the projected tree.
- `src/lib/utils.js`: shared class-name and small UI utility helpers.
- `src/lib/mockData.js`: conceptual conversation, model, tool-schema, role-token,
  and fixture data used by the inspector's mock/design surfaces; it is not the
  source of authoritative runtime state.

### Test-ID contracts

- `src/constants/testIds/auth.js`: shared test IDs for the template auth
  feature set.
- `src/constants/testIds/home.js`: shared test IDs for the template home
  feature set.
- `src/constants/testIds/index.js`: re-exports test-ID groups from one import
  boundary for browser automation.

These files are generic scaffold support. Windie-specific components also use
local `data-testid` values for tree, tool, approval, and onboarding controls.

### Development-server health support

- `plugins/health-check/webpack-health-plugin.js`: Webpack plugin that tracks
  compilation state, errors, warnings, timings, and aggregate health metrics.
- `plugins/health-check/health-endpoints.js`: optional dev-server endpoints for
  detailed health, simple status, readiness, liveness, errors, and compile
  statistics. `craco.config.js` loads this only when
  `ENABLE_HEALTH_CHECK=true`.

### Non-code frontend inputs

- `src/assets/provider-icons/*.svg`: provider icon assets used by extension and
  provider panels; they contain presentation data, not provider behavior.
- `public/index.html`: static document shell, described above.
- `README.md`: Create React App development/build instructions and local
  frontend commands.

The dependency lockfile, generated `build/`, `node_modules/`, coverage output,
and static images/videos are build or presentation inputs rather than frontend
runtime modules.

## State ownership

### `context/WindieContext.jsx`

`WindieProvider` is the frontend application boundary. It owns the state that
multiple UI surfaces need to read or mutate:

- conversation summaries and the active conversation;
- normalized conversation nodes, selected paths, and selected node IDs;
- the separate inspection head used to view a branch;
- sessions, selected session, queued input state, and live-session state;
- transient assistant text, reasoning, and tool-call streaming previews;
- model catalog, model parameters, reasoning options, and token-meter data;
- conversation tool schemas and the available provider tool catalog;
- provider installation status and LLM provider setup state;
- gateway availability, pending approvals, search, theme, overlays, and API
  errors.

Components consume this state through `useWindie()`. Components do not make
arbitrary API calls for shared workflows; they call an operation exposed by the
context.

### Three different kinds of selection

The frontend keeps these concepts separate:

- `selectedNodeId`: the message or tree node currently highlighted for
  inspection and contextual actions.
- `viewHeadId`: an explicit branch head being inspected. It changes the visible
  path without changing the selected session's runtime target.
- `selectedSession.currentHeadMessageId`: the durable runtime head used when a
  session sends a message or continues.

`selectedPathNodes` is derived from the active conversation and the effective
path head. The chat transcript and tree render this path. When a tree node is
clicked, `useSessionRuntime` asks the backend to resolve that conversation/head
pair; only the typed response can select an existing session or expose a new
session view. While that request is pending, `sessionResolution` keeps the UI
in a resolving state instead of presenting a false `new session` state.

### Local browser state

Browser storage is only for UI convenience:

- `windie.treeCollapsed`: whether the conversation tree is collapsed;
- `windie.selected-session:<conversation_id>`: the last selected session for a
  conversation;
None of these values replace backend state. If storage is unavailable, the UI
continues with in-memory state.

## Frontend data model

### Conversation projection

`lib/windieMappers.js` converts an inspection response into the browser's
normalized conversation shape:

```text
conversation
├── id
├── model, reasoning, systemPrompt, toolApprovalMode
├── rootId, rootIds
├── nodes: MessageID -> ConversationNode
│   ├── id
│   ├── parentId
│   ├── childrenIds
│   └── message
│       ├── role
│       ├── parts: text | image
│       └── metadata: tool calls | reasoning | usage | refusal | annotations | audio
├── selectedPath: MessageID[]
├── modelContext
├── latestCompaction
├── paths
└── toolSchemas
```

The backend inspection response is authoritative. The mapper reconstructs
`childrenIds` from each node's `parentId`, converts provider-shaped metadata
into display-shaped metadata, and keeps image assets as references that can be
loaded later.

Conversation summaries are intentionally smaller than inspections. The
conversation picker can list and sort summaries without loading every message;
selecting a conversation loads its inspection snapshot.

### Session projection

`sessionFromApi` converts durable session records into browser names:

```text
session
├── id
├── conversationId
├── startHeadMessageId
├── currentHeadMessageId
├── status
├── model, reasoning
├── queued, queueDepth, queueId
├── latestEventId
├── nodeCount
├── error
└── createdAt, updatedAt
```

Sessions are the only frontend runtime targets. A conversation provides the
tree; a session provides the serialized execution and its current head.

### Pending assistant turn

Streaming output is held separately from persisted conversation nodes:

```text
pendingAssistantBySessionId[session_id]
├── text
├── reasoning
├── toolCalls[index]
│   ├── id
│   ├── name
│   └── argumentsText
└── toolCount
```

The pending value is a display preview. When the backend reports a saved
assistant or tool-result message, the frontend reloads the authoritative
conversation path and clears or resets the preview. A stream preview is never
treated as durable history.

## Transport and mapping boundaries

- `lib/windieApi.js`: the only general HTTP client. It resolves the API base
  URL, parses JSON errors, and exposes typed-ish frontend operations for
  conversations, backend-owned session-head resolution/query/continue,
  sessions, models, tools, providers, gateway state, approvals, and image
  assets.
- `lib/sessionStream.js`: the SSE client for one session's event stream. It
  parses SSE framing and JSON payloads, but does not decide how events affect
  application state.
- `lib/sessionEventCursor.js`: accepts only newer numeric event IDs so replayed
  or duplicated events do not get applied twice.
- `lib/sessionTarget.js`: exposes only small selected-session-head helpers;
  backend APIs decide whether a branch is reused or created.
- `lib/windieMappers.js`: converts API response shapes into frontend models.
  Components consume mapped data rather than backend field names.
- `lib/treeProjection.js`: groups persisted assistant tool-call and tool-result
  nodes into presentation-only execution groups. It never changes the stored
  tree.
- `lib/treeLayout.js`: computes positions and edges for the visual tree. It is
  a layout calculation, not a conversation operation.

The standalone Inspector calls the localhost API directly without an API token.
The default API endpoint is
`http://127.0.0.1:8787`, overridable with `REACT_APP_WINDIE_API_URL` during a
frontend build or by the standalone Inspector's runtime
`WINDIE_API_ADDRESS`/`WINDIE_API_PORT` settings.

## Live session lifecycle

`hooks/useSessionRuntime.js` owns session execution from the browser's point of
view.

1. Load all sessions and hydrate each session's latest event cursor.
2. Load sessions for the active conversation and restore the remembered
   selected session when possible.
3. Resolve tree-selected heads through
   `/api/conversations/:id/sessions/resolve` before changing branch/session
   presentation state. The hook shows a resolving state while the request is
   pending.
4. Subscribe to `/api/sessions/:id/events?after=<cursor>` for every running or
   approval-waiting session.
5. Reduce `assistant_delta`, `reasoning_delta`, and `tool_call_delta` events
   into a transient pending preview.
6. On `input_started`, `assistant_message_saved`, or `tool_result_saved`, reload
   the relevant conversation head and advance the session head.
7. On `completed`, `failed`, `cancelled`, or `waiting_for_approval`, reconcile
   the durable session record, pending preview, and subscription.
8. Abort subscriptions when a session is no longer live, is deleted, or the
   provider unmounts.

The hook keeps async resources in refs: active `AbortController` instances,
event cursors, the latest session records, and the selected session. React
state remains the rendered view; refs prevent stale async callbacks from
overwriting newer selections.

Session queries are serialized by the backend. If a query arrives while a run
is active, the response marks it queued. The frontend displays queue depth and
waits for session events; it does not insert queued input into the conversation
tree itself.

## User action flows

### Initial load

`WindieProvider` loads conversation summaries, gateway status, models, the
available tool catalog, provider installations, and sessions. It selects the
first conversation when none is active. Loading an active conversation fetches
an inspection report and pending approvals in parallel.

### Select a conversation

The picker changes `activeConvId`, clears branch and node selection, and lets
the session runtime load the conversation's selected session head. A latest-
wins sequence number prevents a slower previous inspection request from
replacing a newer selection.

### Send a message

`Composer` collects text and image attachments, model and reasoning choices,
then calls `sendMessage`:

1. Convert text and attachments to API message parts.
2. Send the conversation ID, selected head, and parts to the backend.
3. Let the backend resolve-or-create the durable branch atomically and append
   or queue the input against that branch.
4. Add an optimistic user node when the input starts immediately.
5. Subscribe to session events and show the pending assistant preview.
6. Reload persisted nodes as save events arrive.

The optimistic node exists only to keep the UI responsive. The next inspection
response replaces it with the backend representation.

### Continue a conversation

Continue sends the conversation ID and requested head to the backend. The
backend resolves-or-creates the durable branch and continues it. It does not
mutate the tree until the backend run produces a saved message.

### Inspect or mutate a tree node

The tree and message rows can select a node, set a visible path head, edit a
message, truncate descendants, remove a message, or fork a conversation. The
context sends the mutation to the API, refreshes the affected inspection and
sessions, then restores the best valid head.

Tree execution groups are presentation-only. Context-menu operations always
use the original persisted message ID, never the synthetic group ID.

### Tool approval

The chat surface displays pending approval records immediately above the user
input bar and sends approve or deny actions to the owning session. The
Inspector only controls the conversation's tool access mode. The session
stream remains the source of progress, and a `waiting_for_approval` event
refreshes the authoritative pending approval records. The frontend does not
decide whether a tool call is allowed; that is the backend tool policy.

### Provider and model setup

The tools overlay manages two separate catalogs:

- LLM providers and model keys, which configure model access through the
  backend gateway;
- executable tool providers, whose schemas can be attached to a conversation.

Provider setup actions refresh provider installation status and the persisted
provider tool catalog. Model changes refresh model parameters and conversation
metadata. The Overview tab renders provider README content; the Tools tab
renders the provider schemas returned by the backend.

## Component responsibilities

- `components/windie/TopBar.jsx`: global navigation, conversation picker,
  gateway/model status, tree toggle, theme, tools, and approval indicators.
- `components/windie/Sidebar.jsx`: owns the tree-panel frame.
- `components/windie/TreePanel.jsx`: renders the current projected tree and
  node context menus.
- `components/windie/TreeOverlay.jsx`: renders the expanded tree view and node
  details.
- `components/windie/ConversationPicker.jsx`: lists, filters, creates, selects,
  and deletes conversations.
- `components/windie/ChatPanel.jsx`: renders the selected path as a transcript,
  groups execution steps, handles transcript scrolling, and places the pending
  assistant row.
- `components/windie/MessageRow.jsx`: renders role-specific messages,
  Markdown, images, reasoning, tool metadata, usage, editing, copying, and
  message actions.
- `components/windie/Composer.jsx`: collects text, pasted or selected images,
  model/reasoning settings, send, stop, and continue actions.
- `components/windie/SessionsChip.jsx`: selects and deletes sessions and shows
  status, queue, and branch information.
- `components/windie/InspectorPanel.jsx`: exposes read-only context/tree data,
  system prompt and conversation settings, approvals, tool schemas, provider
  installation controls, and LLM provider setup.
- `components/windie/LlmProvidersPanel.jsx`: configures available LLM
  providers and stores provider keys through the API.
- `components/windie/ExtensionsPanel.jsx`: installs, enables, repairs,
  disables, and uninstalls executable tool providers.
- `components/ui/*`: reusable visual primitives. These components do not own
  Windie domain state.

## Display rules

- The chat panel shows the selected path, not every node in the conversation
  database.
- Assistant tool-call messages and tool results are grouped visually as one
  expandable execution section, while message rows still retain their real
  persisted IDs.
- Reasoning, tool calls, refusals, citations/annotations, audio metadata, and
  token usage are rendered as metadata lanes attached to assistant messages.
- Images are loaded lazily through the authenticated image-asset endpoint and
  represented in messages by asset references rather than embedded database
  bytes.
- Long user messages and tool outputs are collapsed in the browser and can be
  expanded without changing stored content.
- The token meter is valid only when its context signature matches the current
  model, system prompt, tools, compaction state, and selected path.

## Frontend boundary rules

- Durable truth comes from API responses; local state is a cache or a view.
- Only `lib/windieApi.js` knows request paths, HTTP verbs, headers, and API
  authentication.
- Only `lib/sessionStream.js` knows SSE framing and stream transport.
- Only `useSessionRuntime` owns browser subscriptions, event reduction, live
  session reconciliation, and pending-turn previews.
- Only `WindieContext` coordinates shared frontend workflows and reloads.
- Components render state and emit user intent; they do not own durable runtime
  transitions.
- Tree projection and layout may simplify or arrange the display, but cannot
  mutate conversation meaning.
- The frontend never infers tool permission, provider behavior, model context,
  or session serialization rules; it displays backend decisions and sends user
  actions to the corresponding API operation.

## Frontend-to-backend flow

```text
browser action
    ↓
React component
    ↓ useWindie()
WindieContext / useSessionRuntime
    ↓
windieApi.js or sessionStream.js
    ↓ HTTP / SSE
Windie API
    ↓
operation → store / session / runtime / tool provider
    ↓
authoritative response or session event
    ↓
mapper → context state → rendered inspector
```

The browser is therefore an inspector and control surface for the runtime.
Conversation history, execution, permissions, and model context remain backend
responsibilities; the frontend makes those states visible and actionable.
