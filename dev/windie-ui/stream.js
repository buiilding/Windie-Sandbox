// stream.js — parse Windie's session event stream (SSE) without any library.
//
// Why not the browser's built-in EventSource?
//   EventSource cannot set HTTP headers. Windie's API requires the
//   `X-Windie-Api-Token` header on every request, so we use fetch() and read
//   the response body as a stream ourselves.
//
// What this file gives you:
//   streamSessionEvents(sessionId, onEvent, options)
//     - opens the SSE stream for one session
//     - calls onEvent({ id, event, data }) for each event that arrives
//     - resolves when the stream ends, rejects on HTTP or stream failure
//
// The shape of one SSE "block" from the server (see src/api/sse.rs):
//   id: 42
//   event: assistant_delta
//   data: {"type":"assistant_delta","text":"Hello","event_id":42,...}
//
// Blocks are separated by a blank line. We accumulate raw text, split on blank
// lines, and parse each complete block.

// Parse one raw SSE block into { id, event, data }.
// Returns null when the block carries no data (e.g. a keep-alive comment).
function parseSseBlock(block) {
  const lines = block.split(/\r?\n/);
  let id = null;
  let event = "message";
  const dataLines = [];

  for (const line of lines) {
    if (line.startsWith("id:")) {
      id = line.slice(3).trim();
    } else if (line.startsWith("event:")) {
      event = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      // A data line may appear more than once; join them with newlines.
      dataLines.push(line.slice(5).trimStart());
    }
    // Lines starting with ":" are comments (keep-alives) — ignored.
  }

  if (dataLines.length === 0) return null;

  return {
    id,
    event,
    data: JSON.parse(dataLines.join("\n")),
  };
}

// Open one session's event stream and invoke onEvent for each parsed event.
//
//   sessionId  — the session to follow
//   onEvent    — async callback receiving { id, event, data }
//   options.apiBase   — e.g. "http://127.0.0.1:8787"
//   options.token     — Windie API token
//   options.after     — last event id seen; server replays after this cursor
//   options.signal    — AbortSignal so the caller can stop listening
export async function streamSessionEvents(sessionId, onEvent, options = {}) {
  const { apiBase, token, after = null, signal = null } = options;

  // The `after` cursor lets us resume without re-reading events we already saw.
  const cursor = after == null ? "" : `?after=${encodeURIComponent(String(after))}`;
  const url = `${apiBase}/api/sessions/${encodeURIComponent(sessionId)}/events${cursor}`;

  const response = await fetch(url, {
    headers: token ? { "X-Windie-Api-Token": token } : {},
    signal,
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`session stream failed (${response.status}): ${text}`);
  }
  if (!response.body) return;

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  // Read chunks forever. Each chunk may contain partial blocks, so we keep a
  // running buffer and only parse blocks that are terminated by a blank line.
  while (true) {
    const { done, value } = await reader.read();
    buffer += decoder.decode(value || new Uint8Array(), { stream: !done });

    const blocks = buffer.split(/\r?\n\r?\n/);
    buffer = blocks.pop() || ""; // last piece may be incomplete — keep it

    for (const block of blocks) {
      const parsed = parseSseBlock(block.trim());
      if (parsed) await onEvent(parsed);
    }

    if (done) break;
  }

  // Flush any final block that arrived without a trailing blank line.
  const tail = parseSseBlock(buffer.trim());
  if (tail) await onEvent(tail);
}
