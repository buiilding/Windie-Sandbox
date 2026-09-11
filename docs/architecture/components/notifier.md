# Notifier

## Purpose

The notifier is an independent process. It listens for completed-session events
from the Windie API and presents native desktop notifications. It does not run
the session or the model; it only tells the user that a session finished and
shows a short preview of the final assistant response.

## Owns

- The durable aggregate SSE connection to
  `GET /api/events?after=<cursor>&kind=completed`.
- Listening for the `session.completed` event. This is the specific event that
  tells the notifier a session reached its completed state.
- Reading the event's `session_id` and its read-only canonical final assistant
  message projection from the API.
- Creating a notification preview by normalizing the response text and
  limiting it to 240 characters.
- Building the current session link under the configured loopback API origin,
  so a notification click opens that session in the packaged local Inspector
  instead of the anonymous public demo.
- Persisting a completion-event cursor in
  `~/.windie/notifier-completed-event.cursor` so reconnects can resume from the
  last displayed completion.
- In development, listening to the separate volatile
  `notifier.assistant_completed` test signal. This signal only tests native
  notification delivery and is not a real session completion.

## Does not own

- The API's durable session-event records or conversation messages.
- Session execution, model requests, tool calls, approvals, or wakeups.
- The API, Bifrost gateway, or tray process lifecycle.
- The durable meaning of a completion event. The notifier only observes that
  event and presents it.

## Main flow

1. The notifier starts independently and reads its saved completion cursor.
2. It asks the API for the latest completed-event cursor, then connects to the
   aggregate SSE stream using `kind=completed` and the cursor boundary.
3. When the API emits `session.completed`, the notifier reads the session ID
   and final assistant message from the event data.
4. It creates a shortened preview, presents the native notification, and
   associates the notification with that session's Inspector link.
5. After the notification is presented, it saves the event ID and reconnects
   from that point if the stream closes.
6. When the user clicks the notification, the platform opens the session link.

## Important invariants

- Production notifications are triggered only by `session.completed`, not by
  streamed text, tool calls, or every assistant-message update.
- Notification delivery never changes session state or durable conversation
  data.
- The API remains the source of truth. The final response text in the event is
  a read-only projection of the canonical assistant message.
- A persisted cursor prevents already-consumed completions from being replayed
  after a normal reconnect. Because the cursor is saved after presentation, a
  process crash between those operations can present one notification again.
- The development notification signal is volatile and has no replay cursor;
  it can be missed when no notifier is connected.
- The current session action opens the packaged local Inspector URL. It never
  redirects a local completion into the anonymous public demo.

## Related code

- [`src/local/notifier.rs`](../../../src/local/notifier.rs) — independent
  process startup, observer ownership, and shutdown.
- [`src/local/session_event_observer.rs`](../../../src/local/session_event_observer.rs)
  — durable completion stream, cursor persistence, event parsing, and preview
  construction.
- [`src/local/tray_notification.rs`](../../../src/local/tray_notification.rs)
  — platform notification delivery and session-link click actions.
- [`src/api/event.rs`](../../../src/api/event.rs) — aggregate durable event
  cursor and SSE stream.
- [`src/api/sse.rs`](../../../src/api/sse.rs) — `session.completed` event name
  and canonical final-message projection.
- [`src/api/dev.rs`](../../../src/api/dev.rs) — volatile development-only
  notification test signal.
