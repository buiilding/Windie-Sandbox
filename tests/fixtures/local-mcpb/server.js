const readline = require("readline");

const tools = [
  {
    name: "echo",
    description: "Returns the supplied text.",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string" }
      },
      required: ["text"]
    },
    annotations: { readOnlyHint: true }
  }
];

function respond(id, result) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

const input = readline.createInterface({ input: process.stdin });
input.on("line", (line) => {
  const request = JSON.parse(line);
  if (request.method === "notifications/initialized") return;
  if (request.method === "initialize") {
    respond(request.id, {
      protocolVersion: "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "windie-local-mcp-fixture", version: "1.0.0" }
    });
    return;
  }
  if (request.method === "tools/list") {
    respond(request.id, { tools });
    return;
  }
  if (request.method === "tools/call") {
    const text = request.params?.arguments?.text ?? "";
    respond(request.id, {
      content: [{ type: "text", text }]
    });
    return;
  }
  respond(request.id, {
    error: { code: -32601, message: `Unknown method: ${request.method}` }
  });
});
