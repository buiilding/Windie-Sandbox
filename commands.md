# Windie Commands

This file is the concrete CLI command reference. Keep primitive operations here
instead of in `AGENTS.md`.

## No-op

```text
windie
```

Exit successfully without doing anything.

Use this to verify the binary exists without starting chat, creating a
conversation, opening Bifrost, or mutating persisted state.

## Help

```text
windie -h
```

Show help.

```text
windie --help
```

Show help.

Use this to print the current CLI surface and command notes.

## Version

```text
windie -V
```

Show version.

```text
windie --version
```

Show version.

Use this to print the package version compiled into the binary.

## Conversations

```text
windie new
```

Create a new empty conversation.

Output is only the new conversation ID, so the command can be used by scripts.

```text
windie ls
```

List all persisted conversations.

Output includes each conversation ID and message count. If no conversations
exist, it prints `no conversations`.

```text
windie show <conversation_id>
```

Show message previews for one conversation.

Output includes each message role, message ID, and one-line text preview. If the
conversation has no messages, it prints `no messages`.

## Messages

```text
windie append <conversation_id> --role user --text "hello"
```

Append one message to a conversation without querying the model.

The role must currently be one of:

```text
system
user
assistant
tool
```

The command prints the new message ID.

```text
windie update <conversation_id> <message_id> --text "new text"
```

Replace one message's text without querying the model.

This mutates only the selected message content. It does not remove later
messages and does not run inference.

```text
windie rm <conversation_id>
```

Remove one conversation.

This deletes the conversation and all messages/compactions owned by that
conversation.

```text
windie rm <conversation_id> <message_id>
```

Remove one message from a conversation.

This deletes only that message. Remaining child messages are reconnected to the
deleted message's parent.

```text
windie truncate <conversation_id> <message_id>
```

Remove all messages after one message in a conversation.

The checkpoint message is kept. Messages newer than the checkpoint are deleted.

```text
windie fork <conversation_id> <message_id>
```

Create a new conversation copied from the start of an existing conversation
through one message.

The forked conversation receives new message IDs and can diverge independently.
The command prints the new conversation ID.

## Inference

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

The model name is passed to Bifrost, for example `openai/gpt-4o-mini`.

## Runtime Status

```text
windie status
```

Check local runtime and gateway readiness.

This currently reports whether the local Bifrost gateway is running.

## Gateway

```text
windie gateway start
```

Start the local Bifrost gateway.

If the gateway is already running, the command reports that instead of starting
a duplicate process.

```text
windie gateway stop
```

Stop the local Bifrost gateway.

If the gateway is not running, the command reports that instead of failing.

## Benchmarks

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

Use this only when you intentionally want to measure the provider path.
