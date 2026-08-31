# Desktop notifications

## Purpose

The notifier is an optional independent local component that presents a native
notification after a session completes. It is not part of the tray and does not
run models, execute tools, or modify sessions.

## Runtime flow

```text
session completes in API
        │
        v
durable session.completed event in SQLite
        │
        v
notifier reads aggregate API SSE with its component credential
        │
        v
native notification shows a shortened final assistant response
```

The notifier starts from a durable completion cursor stored at
`~/.windie/notifier-completed-event.cursor`. On first start it begins after
already-completed sessions, avoiding a burst of old notifications. On a later
reconnect it resumes after that cursor, so a completion not yet displayed can
be replayed.

Only `session.completed` events generate normal notifications. A failed,
cancelled, or merely streamed response does not. The API projects the canonical
final assistant message for the notifier, which normalizes whitespace and uses
at most 240 characters for the notification body.

## Starting and testing

```text
windie notifier start
windie notifier output
windie notifier stop
```

For foreground development, run:

```text
windie dev run notifier
```

The API also provides a development-only notification probe. It is volatile and
is not evidence that a session completed; production delivery always follows
the durable completion-event stream.

## Notification action

Production completion notifications identify the session they came from. A
native action can open:

```text
https://app.windieos.com/sessions/<session-id>
```

That page still connects to the user's local API and performs normal runtime
access checks. Opening a notification does not resume, cancel, or otherwise
change the session.

On macOS, actionable native notifications require the installed `Windie
Notifier.app` and notification permission. Development fallbacks can display a
notification but do not provide the native click callback. Other supported
platforms use the same session URL action where their notification backend
supports it.

## Related code

- `src/local/notifier.rs`: independent notifier process lifecycle.
- `src/local/session_event_observer.rs`: durable completion observation and
  cursor persistence.
- `src/local/tray_notification.rs`: platform notification presentation and
  session URL action.
- `src/api/event.rs` and `src/api/sse.rs`: aggregate durable event feed and
  final-response projection.
