# Gateway process

## Purpose

The gateway is Bifrost, the local service Windie uses to communicate with LLM
providers. Windie sends it the selected model context, model, reasoning mode,
and tool schemas. Bifrost routes the request to the configured provider and
streams output back to Windie, such as reasoning, normal response text, and
tool calls. Windie normalizes this stream into typed events and stores the
completed response as part of the runtime flow.

Bifrost also provides model-discovery, model-parameter, and input-token-counting
operations used by Windie. By default, the local gateway listens on
`http://localhost:8080`.

## Owns

- Provider communication, including routing requests to the configured LLM
  provider.
- The local OpenAI-compatible model and response endpoints.
- Model discovery, model-parameter metadata, and input-token counting.
- Its own process lifecycle, health check, PID record, and log when Windie
  starts it as a managed component.

## Does not own

- Windie conversations, sessions, messages, or SQLite storage.
- Runtime context selection, tool approval, or tool execution.
- Windie API lifecycle. The API and gateway are independent local processes.
- The Inspector or any other presentation client.

## Main flow

1. Windie starts Bifrost from the local development checkout or the packaged
   Windie-owned binary and waits for its health endpoint.
2. The API compiles the selected model context and sends Bifrost the model,
   reasoning mode, messages, and tool schemas.
3. Bifrost routes the request to the configured provider and streams the
   provider output back to Windie.
4. Windie parses the stream into reasoning, assistant-text, and tool-call
   events, then completes the runtime turn and persists its result.
5. Windie can stop or restart Bifrost independently without changing
   conversations or sessions.

## Important invariants

- The gateway owns provider communication; Windie owns runtime state and
  execution decisions.
- Gateway lifecycle remains independent from the API lifecycle.
- Windie must check gateway readiness before operations that require Bifrost;
  a healthy gateway does not mean a provider, model, or credential is ready.
- The gateway is local by default and uses port `8080`, while
  `WINDIE_GATEWAY_URL` or `WINDIE_GATEWAY_PORT` can provide an explicit
  development configuration.

## Related code

- [`src/llm/gateway.rs`](../../../src/llm/gateway.rs) — Bifrost health checks,
  executable discovery, and managed start/stop behavior.
- [`src/llm/client.rs`](../../../src/llm/client.rs) — OpenAI-compatible
  Responses and input-token HTTP requests.
- [`src/llm/stream.rs`](../../../src/llm/stream.rs) — parses streamed output
  and assembles the completed assistant response.
- [`src/operation/gateway.rs`](../../../src/operation/gateway.rs) — gateway
  status, model metadata, and input-token workflows.
- [`src/dev.rs`](../../../src/dev.rs) — builds and runs the Bifrost source for
  `windie dev run gateway`.
- [`src/local/process.rs`](../../../src/local/process.rs) — shared local
  component process records and logs.
