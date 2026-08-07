# Parallel Search MCP

Windie provides Parallel Search as the `parallel-search` provider through the
hosted Streamable HTTP MCP endpoint:

```text
https://search.parallel.ai/mcp
```

## First-version behavior

- Windie connects directly to Parallel over Streamable HTTP. It does not
  install Node.js, npm, or a local MCP package for this provider.
- Basic anonymous usage is supported by default.
- An optional `PARALLEL_API_KEY` can be stored in `~/.windie/.env` for higher
  rate limits. Windie sends it only as an `Authorization: Bearer` header.
- The provider exposes `web_search` and `web_fetch`.
- Both tools are read-only from Windie's tool metadata perspective, but they
  operate on the open web and therefore require the network permission lane.
- Remote MCP sessions are reused by the API runtime and cleaned up after idle
  shutdown or Windie process shutdown.

## Tools

### `web_search`

Searches the live web using a natural-language objective and returns
LLM-friendly results with URLs, titles, dates, and excerpts.

### `web_fetch`

Extracts relevant content from one or more specific HTTP or HTTPS URLs. It is
intended for reading a page after search results identify it, or when the user
provides a URL directly.

## Authentication

No credential is required for the default endpoint. To configure higher-rate
access, save the optional `PARALLEL_API_KEY` provider secret through the
Inspector or the existing Windie local environment boundary.

Windie does not implement Parallel OAuth in this version. OAuth support belongs
in the generic remote MCP authentication layer and can later use Parallel's
`https://search.parallel.ai/mcp-oauth` endpoint.

## Safety and scope

Installing the provider does not expose its tools to every conversation.
Individual tools still need to be attached to the conversation and approved
by Windie's tool policy. Parallel does not launch a local process, access local
files, or control the user's browser.

See the [official Parallel Search MCP documentation](https://docs.parallel.ai/integrations/mcp/search-mcp)
for the hosted server contract and service limits.
