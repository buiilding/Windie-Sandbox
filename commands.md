# Windie Commands

This file is the concrete CLI command reference. Keep primitive operations here
instead of in `AGENTS.md`.

```text
windie
```

Exit successfully with no runtime action.

```text
windie --help
```

Show help.

```text
windie --version
```

Show version.

```text
windie new
```

Create a new conversation and print its ID.

```text
windie ls
```

List all conversations.

```text
windie show <conversation_id>
```

Show messages in one conversation.

```text
windie append <conversation_id> --role user --text "hello"
```

Append a message to a conversation without querying the model.

```text
windie update <conversation_id> <message_id> --text "new text"
```

Replace one message's text without querying the model.

```text
windie rm <conversation_id>
```

Remove one conversation.

```text
windie rm <conversation_id> <message_id>
```

Remove one message from a conversation.

```text
windie truncate <conversation_id> <message_id>
```

Remove all messages after one message in a conversation.

```text
windie fork <conversation_id> <message_id>
```

Create a new conversation copied from the start of an existing conversation
through one message.

```text
windie query <conversation_id>
```

Run one model response from the conversation and append the assistant message.
Requires the local Bifrost gateway to already be running.

```text
windie query <conversation_id> --model openai/gpt-4o-mini
```

Run one model response using a specific model. Requires the local Bifrost gateway
to already be running.

```text
windie status
```

Check local runtime and gateway readiness.

```text
windie gateway start
```

Start the local Bifrost gateway.

```text
windie gateway stop
```

Stop the local Bifrost gateway.

```text
windie bench
```

Run local/free performance baseline for store open only. Does not start Bifrost
and does not send a provider request.

```text
windie bench <conversation_id>
```

Run local/free performance baseline for one conversation load and model-facing
context build. Does not start Bifrost and does not send a provider request.

```text
windie bench live
```

Run a tiny live provider benchmark. Requires the local Bifrost gateway and sends
a real provider request, so it may cost money.
