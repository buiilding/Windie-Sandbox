const API_BASE =
  (typeof window !== "undefined" && window.__WINDIE_API_URL__) ||
  process.env.REACT_APP_WINDIE_API_URL ||
  "http://127.0.0.1:8787";
function parseApiBody(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return { error: text };
  }
}

function parseSseBlock(block) {
  const lines = block.split(/\r?\n/);
  let id = null;
  let event = "message";
  const data = [];

  for (const line of lines) {
    if (line.startsWith("id:")) {
      id = line.slice("id:".length).trim();
    } else if (line.startsWith("event:")) {
      event = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      data.push(line.slice("data:".length).trimStart());
    }
  }

  if (data.length === 0) return null;

  return {
    id,
    event,
    data: JSON.parse(data.join("\n")),
  };
}

async function streamSse(path, fallbackError, onEvent, options = {}) {
  const response = await fetch(`${API_BASE}${path}`, {
    signal: options.signal,
  });

  if (!response.ok) {
    const text = await response.text();
    const body = parseApiBody(text);
    throw new Error(body?.error || `${fallbackError}: ${response.status}`);
  }

  if (!response.body) return;

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    buffer += decoder.decode(value || new Uint8Array(), { stream: !done });
    const blocks = buffer.split(/\r?\n\r?\n/);
    buffer = blocks.pop() || "";

    for (const block of blocks) {
      const parsed = parseSseBlock(block.trim());
      if (!parsed) continue;
      await onEvent(parsed);
      if (parsed.data?.type === "failed") {
        throw new Error(parsed.data.error || fallbackError);
      }
    }

    if (done) break;
  }

  const final = parseSseBlock(buffer.trim());
  if (final) {
    await onEvent(final);
    if (final.data?.type === "failed") {
      throw new Error(final.data.error || fallbackError);
    }
  }
}

export async function streamSessionEvents(sessionId, afterEventId, onEvent, options = {}) {
  const cursor =
    afterEventId == null ? "" : `?after=${encodeURIComponent(String(afterEventId))}`;
  return streamSse(
    `/api/sessions/${encodeURIComponent(sessionId)}/events${cursor}`,
    "Windie session stream failed",
    onEvent,
    options
  );
}
