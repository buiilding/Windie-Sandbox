// api.js — the small set of Windie API calls v1 needs.
//
// Every request goes through one helper, request(), which:
//   - prefixes the API base URL (http://127.0.0.1:8787)
//   - attaches the X-Windie-Api-Token header
//   - parses the JSON body and throws a readable Error on failure
//
// The token comes from ?windie_token=... in the page URL (this is how
// `windie api` hands it to a browser client) and is remembered in
// localStorage so you don't have to paste it every reload.

const API_BASE = "http://127.0.0.1:8787";
const TOKEN_STORAGE_KEY = "windie_api_token";

// Resolve the API token: URL query param wins, then fall back to localStorage.
export function apiToken() {
  const fromUrl = new URLSearchParams(window.location.search).get("windie_token");
  if (fromUrl) {
    window.localStorage.setItem(TOKEN_STORAGE_KEY, fromUrl);
    return fromUrl;
  }
  return window.localStorage.getItem(TOKEN_STORAGE_KEY) || "";
}

// One helper for every JSON request. Throws Error on non-2xx.
async function request(path, { method = "GET", body = null } = {}) {
  const token = apiToken();
  const response = await fetch(`${API_BASE}${path}`, {
    method,
    headers: {
      ...(body ? { "Content-Type": "application/json" } : {}),
      ...(token ? { "X-Windie-Api-Token": token } : {}),
    },
    body: body ? JSON.stringify(body) : null,
  });

  const text = await response.text();
  const data = text ? JSON.parse(text) : null;

  if (!response.ok) {
    // Windie error bodies look like { error, causes: [...] }.
    throw new Error(data?.error || `request failed (${response.status})`);
  }
  return data;
}

// GET /api/health — no token required. Used to confirm the server is up.
export function health() {
  return fetch(`${API_BASE}/api/health`).then((r) => r.json());
}

// POST /api/conversations — create a new empty conversation. Returns its id.
export function createConversation() {
  return request("/api/conversations", { method: "POST" }).then(
    (d) => d.conversation_id
  );
}

// GET /api/conversations — list all persisted conversations.
// Returns [{ id, title, model, message_count }].
export function listConversations() {
  return request("/api/conversations").then((d) => d.conversations || []);
}

// DELETE /api/conversations/{id} — remove one conversation and its data.
export function deleteConversation(conversationId) {
  return request(`/api/conversations/${conversationId}`, { method: "DELETE" });
}

// POST /api/conversations/{id}/messages — insert a user message as ordered parts.
// parts: [{ type: "text", text } | { type: "image_data", mime_type, data }]
// (image_data = base64, e.g. from a pasted clipboard file — validated by the
// backend in src/input/image.rs, no client-side sniffing needed).
// Returns the new message id (this becomes the head the session runs from).
export function insertUserMessage(conversationId, parts, headMessageId = null) {
  return request(`/api/conversations/${conversationId}/messages`, {
    method: "POST",
    body: {
      head_message_id: headMessageId,
      role: "user",
      parts,
    },
  }).then((d) => d.message_id);
}

// GET /api/conversations/{id}/images/{asset_id} — durable bytes for one image
// part. Returns a Blob; the caller wraps it in URL.createObjectURL for <img>.
export async function fetchImageAsset(conversationId, assetId) {
  const token = apiToken();
  const response = await fetch(
    `${API_BASE}/api/conversations/${conversationId}/images/${encodeURIComponent(assetId)}`,
    { headers: token ? { "X-Windie-Api-Token": token } : {} }
  );
  if (!response.ok) {
    const text = await response.text();
    const data = text ? JSON.parse(text) : null;
    throw new Error(data?.error || `image request failed (${response.status})`);
  }
  return response.blob();
}

// POST /api/conversations/{id}/sessions — create a selectable session branch at
// a head. The branch is "ready" and does not run until queried/continued.
// Returns the new session id.
export function createSession(conversationId, headMessageId = null) {
  return request(`/api/conversations/${conversationId}/sessions`, {
    method: "POST",
    body: { head_message_id: headMessageId, model: null, reasoning: null },
  }).then((d) => d.id);
}

// POST /api/sessions/{id}/query — append a user message to the branch's current
// head and start the run. parts mirrors insertUserMessage; the backend inserts
// them, advances the branch head, and begins streaming. Returns the session.
export function querySession(sessionId, parts) {
  return request(`/api/sessions/${sessionId}/query`, {
    method: "POST",
    body: { parts },
  });
}

// GET /api/models — list models the gateway reports (e.g. your OpenRouter set).
export function listModels() {
  return request("/api/models").then((d) => d.models || []);
}

// PATCH /api/conversations/{id}/model — persist the model for this conversation.
// Sessions started with model:null fall back to this stored conversation model.
export function setConversationModel(conversationId, model) {
  return request(`/api/conversations/${conversationId}/model`, {
    method: "PATCH",
    body: { model },
  });
}

// POST /api/sessions/{id}/approvals/{tool_call_id}/approve
// Execute one pending tool call and let the session continue.
export function approveSessionTool(sessionId, toolCallId) {
  return request(
    `/api/sessions/${sessionId}/approvals/${encodeURIComponent(toolCallId)}/approve`,
    { method: "POST", body: {} }
  );
}

// POST /api/sessions/{id}/approvals/{tool_call_id}/deny
// Store a rejected result for one pending tool call and let the session continue.
export function denySessionTool(sessionId, toolCallId) {
  return request(
    `/api/sessions/${sessionId}/approvals/${encodeURIComponent(toolCallId)}/deny`,
    { method: "POST", body: {} }
  );
}

// GET /api/tools — the full provider tool catalog + provider availability.
export function listTools() {
  return request("/api/tools");
}

// GET /api/conversations/{id}/tools — this conversation's attached tools (light).
export function listAttachedTools(conversationId) {
  return request(`/api/conversations/${conversationId}/tools`);
}

// GET /api/conversations/{id} — the full inspection report.
// Includes messages (with parts + metadata), path, model, tool_schemas, etc.
// This is the source of truth used to render the persisted conversation.
export function getConversation(conversationId, headMessageId = null) {
  const query = headMessageId ? `?head_message_id=${encodeURIComponent(headMessageId)}` : "";
  return request(`/api/conversations/${conversationId}${query}`);
}

// GET /api/sessions/{id}/approvals — the session's authoritative pending
// tool approvals, computed by the backend from store + policy. This is the
// source of truth for which tool calls still await a decision.
export function listSessionApprovals(sessionId) {
  return request(`/api/sessions/${sessionId}/approvals`);
}

// POST /api/conversations/{id}/tools — attach one provider tool.
// providerToolName is the provider-native name (e.g. "read_file"), not the
// namespaced schema name ("desktop_commander__read_file") that Windie stores.
export function attachTool(conversationId, providerId, providerToolName) {
  return request(`/api/conversations/${conversationId}/tools`, {
    method: "POST",
    body: { provider_id: providerId, tool_name: providerToolName },
  });
}

// POST /api/conversations/{id}/tools/batch — attach several provider tools at once.
// tools: [{ providerId, providerToolName }]
export function attachTools(conversationId, tools) {
  return request(`/api/conversations/${conversationId}/tools/batch`, {
    method: "POST",
    body: {
      tools: tools.map((t) => ({ provider_id: t.providerId, tool_name: t.providerToolName })),
    },
  });
}

// DELETE /api/conversations/{id}/tools/{schema_name} — detach by namespaced name.
export function detachTool(conversationId, schemaName) {
  return request(
    `/api/conversations/${conversationId}/tools/${encodeURIComponent(schemaName)}`,
    { method: "DELETE" }
  );
}

// PATCH /api/conversations/{id}/tool-approval-mode — "manual" | "auto_approve_attached".
export function setToolApprovalMode(conversationId, mode) {
  return request(`/api/conversations/${conversationId}/tool-approval-mode`, {
    method: "PATCH",
    body: { mode },
  });
}
