# Windie architecture overview

Windie is a local AI runtime. Its core job is to take a wakeup—most often a
user message—continue the selected conversation branch, and safely persist the
resulting work. The browser is a client of that runtime, not the place where
the work runs.

```text
Inspector action or another wakeup
                │
                v
       Windie API resolves a session
                │
                v
   runtime builds context from one tree path
                │
                v
        Bifrost sends the request to a model
                │
                v
API saves messages, session state, and durable events in SQLite
                │
                ├── Inspector renders updates
                └── notifier can show completion
```

## The core model

The **conversation tree** is the durable message history. A selected
root-to-head path through that tree is the transcript for one model request.
A **session** is a durable execution record that points at a branch head; it
tracks whether work is running, waiting for approval, finished, or able to
continue. It does not copy the conversation.

A **runtime turn** advances a session from an input or wakeup. It compiles a
fresh model request from durable state, saves the assistant response, runs or
requests approval for any tool calls, saves tool results, and repeats until it
reaches an outcome.

## Authority boundaries

The API is the authority for session resolution and runtime execution. SQLite
is the durable record of conversations, sessions, and events. The Inspector
displays API snapshots and sends user actions, but it does not infer session
ownership, build model context, execute tools, or keep a session alive.

Bifrost is the provider boundary: it routes Windie's OpenAI-compatible model
requests to configured LLM providers. Plugins and MCP components supply
optional capabilities, while Windie's runtime and approval policy remain in
control of whether a requested tool runs.

## Read next

- [Storage](storage/)
- [Execution](execution/)
- [LLM](llm/)
- [Extensions](extensions/)
- [Interfaces](interfaces/)
- [Components](components/)
