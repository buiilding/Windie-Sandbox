/**
 * Windie mock data.
 * Conceptual data types:
 *   Conversation: { id, name, model, systemPrompt, rootId, nodes: {id -> Node}, selectedPath: [id...], updatedAt, tags }
 *   Node: { id, parentId, childrenIds: [id], message: Message }
 *   Message: { role: 'system'|'user'|'assistant'|'tool', parts: [Part], metadata?: AssistantMetadata, model?, timestamp, tokens? }
 *   Part: { type: 'text'|'image', text?, url?, alt? }
 *   AssistantMetadata: { toolCalls?, reasoning?, refusal?, annotations?, audio? }
 *   ToolSchema: { name, description, parameters }
 */

export const MODELS = [
  { id: "windie-local-8b", label: "windie-local-8b", family: "local" },
  { id: "windie-local-32b", label: "windie-local-32b", family: "local" },
  { id: "gpt-5.2", label: "gpt-5.2", family: "cloud" },
  { id: "claude-sonnet-4.6", label: "claude-sonnet-4.6", family: "cloud" },
  { id: "gemini-3-pro", label: "gemini-3-pro", family: "cloud" },
];

export const TOOL_SCHEMAS = [
  {
    name: "fs.read_file",
    description: "Read a file from the local filesystem.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Absolute file path" },
        encoding: { type: "string", enum: ["utf-8", "binary"], default: "utf-8" },
      },
      required: ["path"],
    },
  },
  {
    name: "fs.list_dir",
    description: "List directory contents.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string" },
        depth: { type: "integer", minimum: 1, default: 1 },
      },
      required: ["path"],
    },
  },
  {
    name: "runtime.query",
    description: "Issue an explicit runtime query against a subgraph.",
    parameters: {
      type: "object",
      properties: {
        subgraph: { type: "string" },
        prompt: { type: "string" },
        max_tokens: { type: "integer", default: 512 },
      },
      required: ["subgraph", "prompt"],
    },
  },
  {
    name: "vec.search",
    description: "Vector search a local corpus.",
    parameters: {
      type: "object",
      properties: {
        corpus: { type: "string" },
        query: { type: "string" },
        k: { type: "integer", default: 5 },
      },
      required: ["corpus", "query"],
    },
  },
];

const uid = (() => {
  let n = 1000;
  return (p = "n") => `${p}_${(++n).toString(36)}`;
})();

/**
 * helper to build a linear chain of nodes and return
 * { rootId, nodes, path }
 */
function chain(messages) {
  const nodes = {};
  let prevId = null;
  const path = [];
  messages.forEach((m) => {
    const id = uid();
    nodes[id] = {
      id,
      parentId: prevId,
      childrenIds: [],
      message: m,
    };
    if (prevId) nodes[prevId].childrenIds.push(id);
    path.push(id);
    prevId = id;
  });
  return { rootId: path[0], nodes, path };
}

/** Add a branch to an existing tree at a given parent node. Returns new path segment ids. */
function branchFrom(nodes, parentId, messages) {
  let prev = parentId;
  const ids = [];
  messages.forEach((m) => {
    const id = uid();
    nodes[id] = { id, parentId: prev, childrenIds: [], message: m };
    nodes[prev].childrenIds.push(id);
    ids.push(id);
    prev = id;
  });
  return ids;
}

// --- Conversation 1: local runtime bring-up (multi-branch, tool calls, image) ---
const c1 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [
        {
          type: "text",
          text: "You are Windie, a local AI runtime primitive. Respond concisely. When asked to inspect files, use fs.* tools. Prefer plain text over markdown flourishes.",
        },
      ],
      timestamp: "2026-02-04T10:12:04Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "sketch the architecture for a local-first runtime that lets me fork any conversation node and re-query with a different model.",
        },
      ],
      timestamp: "2026-02-04T10:12:11Z",
    },
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [
        {
          type: "text",
          text: "Core primitive: ConversationTree. Every message is a node with a parent and 0..n children. The runtime never mutates history — forks create sibling branches. An SelectedPath is just an ordered list of node ids selected from the tree; the model context is derived from that path at query time.\n\nMinimal surface:\n  • append(nodeId, message) -> nodeId\n  • fork(nodeId) -> nodeId (new sibling parent)\n  • select_selected_path([nodeIds])\n  • query(path) -> stream",
        },
      ],
      metadata: {
        reasoning:
          "User wants a runtime shape, not a product pitch. Keep it primitive-first. Emphasize immutability of history + explicit selected path selection. Avoid mentioning tool execution — out of scope.",
        annotations: [
          { label: "primitive", note: "ConversationTree is the atom" },
          { label: "invariant", note: "history is append-only; forks == siblings" },
        ],
      },
      tokens: 184,
      timestamp: "2026-02-04T10:12:14Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "here's the diagram I was iterating on earlier — does the shape line up with what you described?",
        },
        {
          type: "image",
          url: "https://images.unsplash.com/photo-1571666521805-f5e8423aba9d?crop=entropy&cs=srgb&fm=jpg&ixid=M3w4NjA2OTV8MHwxfHNlYXJjaHwxfHxhcmNoaXRlY3R1cmUlMjBkaWFncmFtfGVufDB8fHx8MTc4MzE5NTcwN3ww&ixlib=rb-4.1.0&q=85",
          alt: "architecture-sketch.png",
        },
      ],
      timestamp: "2026-02-04T10:14:02Z",
    },
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [
        {
          type: "text",
          text: "Roughly, yes. The tree/path split is right. One correction: your 'context assembler' block belongs inside query(), not as a persistent store. The tree is the store; assembly is a pure function of (tree, selectedPath, systemPrompt).",
        },
      ],
      metadata: {
        reasoning:
          "Image is a whiteboard-style diagram. Confirm the shape but call out the one design leak — the assembler shouldn't be stateful.",
      },
      tokens: 82,
      timestamp: "2026-02-04T10:14:07Z",
    },
    {
      role: "user",
      parts: [{ type: "text", text: "peek at the current runtime config on disk." }],
      timestamp: "2026-02-04T10:15:41Z",
    },
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [{ type: "text", text: "" }],
      metadata: {
        toolCalls: [
          {
            id: "call_01",
            name: "fs.read_file",
            arguments: { path: "/etc/windie/runtime.toml", encoding: "utf-8" },
            status: "ok",
          },
        ],
        reasoning:
          "User asked to peek. Single fs.read_file is sufficient; no need to list_dir first.",
      },
      tokens: 24,
      timestamp: "2026-02-04T10:15:42Z",
    },
    {
      role: "tool",
      parts: [
        {
          type: "text",
          text: '# /etc/windie/runtime.toml\n[runtime]\nmodel_default = "windie-local-32b"\nmax_selected_path = 128\nfork_policy   = "copy-on-branch"\n\n[storage]\nbackend = "sqlite"\npath    = "~/.windie/tree.db"\n\n[tools]\nenabled = ["fs.read_file","fs.list_dir","vec.search"]',
        },
      ],
      metadata: { toolCallId: "call_01", toolName: "fs.read_file" },
      timestamp: "2026-02-04T10:15:42Z",
    },
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [
        {
          type: "text",
          text: "Runtime is on sqlite with copy-on-branch forks. runtime.query isn't enabled in tools — you'll want to add it before experimenting with subgraph queries.",
        },
      ],
      metadata: {
        annotations: [
          { label: "config-gap", note: "runtime.query missing from tools.enabled" },
        ],
      },
      tokens: 44,
      timestamp: "2026-02-04T10:15:44Z",
    },
  ]);
  // Add a branch alt to the assistant reply on the architecture question (index 2)
  const arcAssistantId = built.path[2];
  const branchIds = branchFrom(built.nodes, built.nodes[arcAssistantId].parentId, [
    {
      role: "assistant",
      model: "gpt-5.2",
      parts: [
        {
          type: "text",
          text: "Think of the runtime as three pure functions over one data structure:\n  tree           : Map<NodeId, Node>\n  select_path    : (tree, leafId) -> [NodeId]\n  render_context : (tree, path, systemPrompt) -> ModelRequest\n\nEverything else — fork, edit, truncate — is a tree mutation. The 'chat UI' is just a projection of one selected path.",
        },
      ],
      metadata: {
        reasoning:
          "Alternative framing requested by user via fork. Lean into functional shape.",
        annotations: [
          { label: "alternative", note: "same primitive, more functional framing" },
        ],
      },
      tokens: 148,
      timestamp: "2026-02-04T10:13:02Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "I like this framing better. how would edit work without breaking history?",
        },
      ],
      timestamp: "2026-02-04T10:13:20Z",
    },
    {
      role: "assistant",
      model: "gpt-5.2",
      parts: [
        {
          type: "text",
          text: "edit(nodeId, newMessage) creates a sibling node with the new content and re-points the selected path to it. The original node is still in the tree, just no longer on the selected path. Nothing is destructive.",
        },
      ],
      tokens: 61,
      timestamp: "2026-02-04T10:13:24Z",
    },
  ]);

  return {
    id: "conv_arch",
    name: "runtime primitive: tree + selected path",
    model: "windie-local-32b",
    systemPrompt:
      "You are Windie, a local AI runtime primitive. Respond concisely. When asked to inspect files, use fs.* tools. Prefer plain text over markdown flourishes.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    alternatePath: [built.path[0], built.path[1], ...branchIds],
    updatedAt: "2026-02-04T10:15:44Z",
    tags: ["runtime", "design"],
  };
})();

// --- Conversation 2: debugging streaming tokens (short, tool-heavy) ---
const c2 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [
        {
          type: "text",
          text: "You are Windie. When the user asks for diagnostics, prefer fs.list_dir + fs.read_file. Do not invent file paths.",
        },
      ],
      timestamp: "2026-02-03T18:02:11Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "streaming stalls after ~40 tokens on windie-local-8b. where should I look first?",
        },
      ],
      timestamp: "2026-02-03T18:02:20Z",
    },
    {
      role: "assistant",
      model: "windie-local-8b",
      parts: [{ type: "text", text: "" }],
      metadata: {
        toolCalls: [
          {
            id: "call_11",
            name: "fs.list_dir",
            arguments: { path: "~/.windie/logs", depth: 1 },
            status: "ok",
          },
        ],
      },
      timestamp: "2026-02-03T18:02:22Z",
    },
    {
      role: "tool",
      parts: [
        {
          type: "text",
          text: "stream-2026-02-03.log     412K\nruntime-2026-02-03.log    91K\ntool-2026-02-03.log        7K",
        },
      ],
      metadata: { toolCallId: "call_11", toolName: "fs.list_dir" },
      timestamp: "2026-02-03T18:02:22Z",
    },
    {
      role: "assistant",
      model: "windie-local-8b",
      parts: [
        {
          type: "text",
          text: "Two candidates. Start with the stream log — a 40-token stall usually shows up as a widening gap between chunk timestamps. If that log looks clean, the runtime log will show whether the scheduler starved the decode loop.",
        },
      ],
      metadata: {
        reasoning:
          "Two log files worth reading. Rank by likelihood: stream log first (direct signal), runtime log second (scheduling).",
      },
      tokens: 71,
      timestamp: "2026-02-03T18:02:26Z",
    },
  ]);
  return {
    id: "conv_stream",
    name: "streaming stall @ ~40 tokens",
    model: "windie-local-8b",
    systemPrompt:
      "You are Windie. When the user asks for diagnostics, prefer fs.list_dir + fs.read_file. Do not invent file paths.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    updatedAt: "2026-02-03T18:02:26Z",
    tags: ["debug", "streaming"],
  };
})();

// --- Conversation 3: refusal + reasoning ---
const c3 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [
        {
          type: "text",
          text: "You are Windie. Refuse anything that requires unattended remote execution — this runtime is local-only by design.",
        },
      ],
      timestamp: "2026-02-02T09:44:00Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "spin up an ssh session to my staging box and tail the logs for me.",
        },
      ],
      timestamp: "2026-02-02T09:44:14Z",
    },
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [
        {
          type: "text",
          text: "I can't do that here. This runtime intentionally has no remote-shell surface; unattended ssh would violate the local-only invariant. If you paste the tail output I can help you read it.",
        },
      ],
      metadata: {
        refusal: {
          category: "out-of-scope-capability",
          reason:
            "Remote shell/session capability is intentionally not exposed by the local runtime.",
        },
        reasoning:
          "User requested remote execution. Refuse cleanly, offer an in-scope fallback (paste the log).",
      },
      tokens: 58,
      timestamp: "2026-02-02T09:44:17Z",
    },
  ]);
  return {
    id: "conv_refuse",
    name: "refused: remote ssh request",
    model: "windie-local-32b",
    systemPrompt:
      "You are Windie. Refuse anything that requires unattended remote execution — this runtime is local-only by design.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    updatedAt: "2026-02-02T09:44:17Z",
    tags: ["refusal"],
  };
})();

// --- Conversation 4: annotations + audio ---
const c4 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [
        {
          type: "text",
          text: "You are Windie. When summarizing audio, keep timestamps and speakers explicit.",
        },
      ],
      timestamp: "2026-02-01T14:20:00Z",
    },
    {
      role: "user",
      parts: [
        {
          type: "text",
          text: "summarize the standup recording I attached this morning.",
        },
      ],
      timestamp: "2026-02-01T14:20:07Z",
    },
    {
      role: "assistant",
      model: "gemini-3-pro",
      parts: [
        {
          type: "text",
          text: "Three things landed:\n  1) tree persistence is behind a feature flag, needs review before Friday.\n  2) fork-from-node UI merged, but truncate-after is still open.\n  3) reasoning-trace surfacing needs a design pass — engineers want it collapsed by default.",
        },
      ],
      metadata: {
        annotations: [
          {
            label: "action-item",
            note: "review tree-persistence flag before Fri",
            owner: "@mira",
          },
          {
            label: "open",
            note: "truncate-after-node UI still pending",
          },
          {
            label: "design-request",
            note: "reasoning-trace surfacing collapsed default",
          },
        ],
        audio: {
          durationSec: 842,
          transcriptTokens: 5120,
          speakers: 4,
          source: "standup-2026-02-01.m4a",
        },
      },
      tokens: 132,
      timestamp: "2026-02-01T14:20:11Z",
    },
  ]);
  return {
    id: "conv_audio",
    name: "standup summary — 2026-02-01",
    model: "gemini-3-pro",
    systemPrompt:
      "You are Windie. When summarizing audio, keep timestamps and speakers explicit.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    updatedAt: "2026-02-01T14:20:11Z",
    tags: ["audio", "summary"],
  };
})();

// --- Conversation 5: long-running, multiple forks ---
const c5 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [
        {
          type: "text",
          text: "You are Windie. The user is writing a spec. Push back when the spec is ambiguous.",
        },
      ],
      timestamp: "2026-01-31T11:00:00Z",
    },
    {
      role: "user",
      parts: [
        { type: "text", text: "help me draft the spec for RuntimeRequestPreview." },
      ],
      timestamp: "2026-01-31T11:00:12Z",
    },
    {
      role: "assistant",
      model: "claude-sonnet-4.6",
      parts: [
        {
          type: "text",
          text: "One-liner: RuntimeRequestPreview is the deterministic projection of (tree, selectedPath, systemPrompt, conversation model) into the exact payload the runtime would send. It is a *preview* — it must never mutate state.\n\nMinimum fields:\n  • model             (persisted conversation model)\n  • messages          (flattened selected path)\n  • tools             (enabled tool schemas)\n  • token_budget      (remaining budget after path)\n  • hash              (stable content hash for reproducibility)",
        },
      ],
      metadata: {
        reasoning:
          "Frame it as a pure projection to head off future 'why did the preview mutate my tree' bugs.",
      },
      tokens: 176,
      timestamp: "2026-01-31T11:00:16Z",
    },
    {
      role: "user",
      parts: [{ type: "text", text: "should the hash include the systemPrompt?" }],
      timestamp: "2026-01-31T11:01:04Z",
    },
    {
      role: "assistant",
      model: "claude-sonnet-4.6",
      parts: [
        {
          type: "text",
          text: "Yes. Two runs with the same path and different system prompts must not collide. Hash over (systemPrompt, messages, tools, model). Exclude token_budget — that's advisory, not part of identity.",
        },
      ],
      tokens: 62,
      timestamp: "2026-01-31T11:01:07Z",
    },
  ]);
  // add a sibling branch to the last assistant message
  const lastAssistant = built.path[built.path.length - 1];
  branchFrom(built.nodes, built.nodes[lastAssistant].parentId, [
    {
      role: "assistant",
      model: "windie-local-32b",
      parts: [
        {
          type: "text",
          text: "Alternative: don't hash at all. RuntimeRequestPreview is ephemeral by definition — if you want reproducibility, snapshot the node ids instead. Cheaper, and it survives model swaps.",
        },
      ],
      metadata: {
        annotations: [{ label: "counterproposal", note: "node-id snapshot vs. content hash" }],
      },
      tokens: 54,
      timestamp: "2026-01-31T11:01:22Z",
    },
  ]);
  return {
    id: "conv_spec",
    name: "spec: RuntimeRequestPreview",
    model: "claude-sonnet-4.6",
    systemPrompt:
      "You are Windie. The user is writing a spec. Push back when the spec is ambiguous.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    updatedAt: "2026-01-31T11:01:07Z",
    tags: ["spec"],
  };
})();

// --- Conversation 6: quick note ---
const c6 = (() => {
  const built = chain([
    {
      role: "system",
      parts: [{ type: "text", text: "You are Windie. Answer in ≤2 lines." }],
      timestamp: "2026-01-30T08:12:00Z",
    },
    {
      role: "user",
      parts: [{ type: "text", text: "canonical extension for tree exports?" }],
      timestamp: "2026-01-30T08:12:05Z",
    },
    {
      role: "assistant",
      model: "windie-local-8b",
      parts: [{ type: "text", text: ".wtree — JSONL, one node per line, parent id first." }],
      tokens: 18,
      timestamp: "2026-01-30T08:12:06Z",
    },
  ]);
  return {
    id: "conv_ext",
    name: "canonical tree export extension",
    model: "windie-local-8b",
    systemPrompt: "You are Windie. Answer in ≤2 lines.",
    rootId: built.rootId,
    nodes: built.nodes,
    selectedPath: built.path,
    updatedAt: "2026-01-30T08:12:06Z",
    tags: ["misc"],
  };
})();

export const INITIAL_CONVERSATIONS = [c1, c2, c3, c4, c5, c6];

export const ROLE_TOKENS = {
  system: { label: "SYS", color: "text-muted-foreground" },
  user: { label: "USR", color: "text-[hsl(var(--user-message))]" },
  assistant: { label: "AST", color: "text-[hsl(var(--assistant-message))]" },
  tool: { label: "TOL", color: "text-[hsl(var(--tool-call))]" },
};
