// app.js — the single v1 screen. Wires api.js + stream.js into a send→stream loop.
//
// Flow when you hit send:
//   1. insertUserMessage  (api.js)  — save your message, get a message id
//   2. createSession      (api.js)  — start a session from that head, get a session id
//   3. streamSessionEvents(stream.js) — listen and append assistant tokens live
//
// State is deliberately tiny: one conversation id, one current head message id.

import {
  health,
  createConversation,
  insertUserMessage,
  createSession,
  listModels,
  setConversationModel,
  approveSessionTool,
  denySessionTool,
  listSessionApprovals,
  listTools,
  listAttachedTools,
  getConversation,
  attachTool,
  attachTools,
  detachTool,
  setToolApprovalMode,
} from "./api.js";
import { streamSessionEvents } from "./stream.js";

const API_BASE = "http://127.0.0.1:8787";

// --- tiny state ---
let conversationId = null;   // which conversation we're in
let headMessageId = null;    // the message the next session runs from

// --- element handles ---
const statusEl = document.getElementById("status");
const messagesEl = document.getElementById("messages");
const inputEl = document.getElementById("input");
const sendEl = document.getElementById("send");

// model picker elements
const modelBtn = document.getElementById("model-btn");          // composer button
const modelBtnName = document.getElementById("model-btn-name"); // text on the button
const overlayEl = document.getElementById("overlay");           // dim backdrop
const pickerEl = document.getElementById("picker");             // the panel
const pickerRefresh = document.getElementById("picker-refresh");
const pickerFilter = document.getElementById("picker-filter");
const pickerList = document.getElementById("picker-list");

// --- model picker state ---
let allModels = [];        // every model id from the gateway
let selectedModel = null;  // the model currently applied to the conversation

// Add a chat bubble; returns the element so we can append streamed text to it.
function addMessage(role, label) {
  const div = document.createElement("div");
  div.className = `msg ${role}`;
  if (label) {
    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = label;
    div.appendChild(meta);
  }
  const body = document.createElement("div");
  div.appendChild(body);
  messagesEl.appendChild(div);
  messagesEl.scrollTop = messagesEl.scrollHeight;
  return body;
}

// The provider segment of a model id, shown as the right-hand tag.
// "openrouter/moonshotai/kimi-k3" -> "OPENROUTER"
function providerTag(modelId) {
  const provider = modelId.split("/")[0] || "";
  return provider.toUpperCase();
}

// Fetch all model ids and keep them for filtering.
async function loadModels() {
  try {
    const models = await listModels();
    allModels = models.map((m) => m.id);
    renderPickerList();
  } catch (error) {
    statusEl.textContent = `failed to load models: ${error.message}`;
  }
}

// Render the picker rows matching the current filter text.
function renderPickerList() {
  const query = pickerFilter.value.trim().toLowerCase();
  const matches = allModels.filter((id) => id.toLowerCase().includes(query));
  pickerList.innerHTML = "";
  for (const id of matches) {
    const row = document.createElement("div");
    row.className = "prow" + (id === selectedModel ? " sel" : "");

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = id;

    const provider = document.createElement("span");
    provider.className = "provider";
    provider.textContent = providerTag(id);

    row.appendChild(name);
    row.appendChild(provider);
    row.addEventListener("click", () => pickModel(id));
    pickerList.appendChild(row);
  }
}

// Apply a chosen model to this conversation, update the button, close the panel.
async function pickModel(id) {
  selectedModel = id;
  modelBtnName.textContent = id;
  closePicker();
  if (conversationId) await setConversationModel(conversationId, id);
  statusEl.textContent = `conversation ${conversationId.slice(0, 8)}… · ${id}`;
}

function openPicker() {
  overlayEl.classList.add("open");
  pickerEl.classList.add("open");
  pickerFilter.value = "";
  renderPickerList();
  pickerFilter.focus();
}

function closePicker() {
  overlayEl.classList.remove("open");
  pickerEl.classList.remove("open");
}

modelBtn.addEventListener("click", openPicker);
overlayEl.addEventListener("click", () => {
  closePicker();
  closeTools();
});
pickerRefresh.addEventListener("click", loadModels);
pickerFilter.addEventListener("input", renderPickerList);

// ===========================================================================
// Tools panel — attach/detach provider tools, mirroring the inspector.
// ===========================================================================

const toolsBtn = document.getElementById("tools-btn");
const toolsBtnCount = document.getElementById("tools-btn-count");
const toolsEl = document.getElementById("tools");
const toolsList = document.getElementById("tools-list");
const modeManualBtn = document.getElementById("mode-manual");
const modeAutoBtn = document.getElementById("mode-auto");

// Tools panel state.
let toolCatalog = [];          // all available provider tools (ToolDefinition)
let toolProviders = [];        // provider availability statuses
let attachedNames = new Set(); // namespaced schema names attached to this conversation
let approvalMode = "manual";   // "manual" | "auto_approve_attached"
const collapsedProviders = new Set(); // providerIds currently collapsed
const pendingActions = new Set();     // in-flight attach/detach keys

// Human label for a provider id (matches the inspector).
function providerLabel(providerId) {
  if (providerId === "windie") return "Windie";
  if (providerId === "cua-driver") return "CUA Driver";
  if (providerId === "desktop-commander") return "Desktop Commander";
  if (providerId === "blender-mcp") return "Blender MCP";
  if (providerId === "brightdata") return "Bright Data";
  return providerId || "Unknown Provider";
}

// Group the catalog by provider id -> [{ providerId, tools: [...] }].
function groupByProvider(tools) {
  const groups = [];
  const byId = new Map();
  for (const tool of tools) {
    const providerId = tool.provider?.provider_id || "unknown";
    let group = byId.get(providerId);
    if (!group) {
      group = { providerId, tools: [] };
      byId.set(providerId, group);
      groups.push(group);
    }
    group.tools.push(tool);
  }
  return groups;
}

// Load the catalog and the conversation's attached tools, then render.
async function loadToolsPanel() {
  if (!conversationId) return;
  try {
    const [catalog, attached] = await Promise.all([
      listTools(),
      listAttachedTools(conversationId),
    ]);
    toolCatalog = catalog.tools || [];
    toolProviders = catalog.providers || [];
    attachedNames = new Set((attached.tools || []).map((t) => t.name));
    toolsBtnCount.textContent = String(attachedNames.size);
    renderToolsList();
  } catch (error) {
    statusEl.textContent = `failed to load tools: ${error.message}`;
  }
}

// Run one attach/detach action with a pending guard, then refresh the panel.
async function runToolAction(key, action) {
  if (pendingActions.has(key)) return;
  pendingActions.add(key);
  renderToolsList(); // show disabled/spinner state
  try {
    await action();
  } catch (error) {
    statusEl.textContent = `tool action failed: ${error.message}`;
  } finally {
    pendingActions.delete(key);
  }
  // Reload authoritative attached state from the server.
  const attached = await listAttachedTools(conversationId);
  attachedNames = new Set((attached.tools || []).map((t) => t.name));
  toolsBtnCount.textContent = String(attachedNames.size);
  renderToolsList();
}

function iconButton(label, danger, disabled, onClick) {
  const btn = document.createElement("button");
  btn.className = "iconbtn" + (danger ? " danger" : "");
  btn.textContent = label;
  btn.disabled = disabled;
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  return btn;
}

// Render the whole provider-card list from current state.
function renderToolsList() {
  toolsList.innerHTML = "";

  // Reflect the current approval mode on the toggle.
  modeManualBtn.classList.toggle("on", approvalMode === "manual");
  modeAutoBtn.classList.toggle("on", approvalMode === "auto_approve_attached");

  const groups = groupByProvider(toolCatalog);
  const unavailable = toolProviders.filter((p) => !p.available);

  for (const group of groups) {
    const { providerId, tools } = group;
    const attached = tools.filter((t) => attachedNames.has(t.name));
    const unattached = tools.filter((t) => !attachedNames.has(t.name));
    const addKey = `provider:add:${providerId}`;
    const removeKey = `provider:remove:${providerId}`;
    const providerPending = pendingActions.has(addKey) || pendingActions.has(removeKey);
    const collapsed = collapsedProviders.has(providerId);

    const card = document.createElement("div");
    card.className = "pcard";

    // header (click to collapse/expand)
    const head = document.createElement("div");
    head.className = "phead";
    const left = document.createElement("div");
    const pname = document.createElement("div");
    pname.className = "pname";
    pname.textContent = providerLabel(providerId);
    const pcount = document.createElement("div");
    pcount.className = "pcount";
    pcount.textContent = `${tools.length} tool${tools.length === 1 ? "" : "s"}`;
    left.appendChild(pname);
    left.appendChild(pcount);
    const acts = document.createElement("div");
    acts.className = "acts";
    if (unattached.length > 0) {
      acts.appendChild(iconButton("+", false, providerPending, () =>
        runToolAction(addKey, () =>
          attachTools(
            conversationId,
            unattached.map((t) => ({
              providerId: t.provider.provider_id,
              providerToolName: t.provider.tool_name,
            }))
          )
        )
      ));
    }
    if (attached.length > 0) {
      acts.appendChild(iconButton("🗑", true, providerPending, () =>
        runToolAction(removeKey, async () => {
          for (const t of attached) await detachTool(conversationId, t.name);
        })
      ));
    }
    head.appendChild(left);
    head.appendChild(acts);
    head.addEventListener("click", () => {
      if (collapsedProviders.has(providerId)) collapsedProviders.delete(providerId);
      else collapsedProviders.add(providerId);
      renderToolsList();
    });
    card.appendChild(head);

    // individual tool rows (when expanded)
    if (!collapsed) {
      for (const tool of tools) {
        const isAttached = attachedNames.has(tool.name);
        const displayName = tool.provider?.tool_name || tool.name;
        const row = document.createElement("div");
        row.className = "prow2";
        const text = document.createElement("div");
        const tname = document.createElement("div");
        tname.className = "tname";
        tname.textContent = displayName;
        const tdesc = document.createElement("div");
        tdesc.className = "tdesc";
        tdesc.textContent = tool.description || "";
        text.appendChild(tname);
        text.appendChild(tdesc);
        const rowActs = document.createElement("div");
        rowActs.className = "acts";
        const toolKey = `tool:${isAttached ? "remove" : "add"}:${tool.name}`;
        const disabled = providerPending || pendingActions.has(toolKey);
        rowActs.appendChild(
          iconButton(isAttached ? "🗑" : "+", isAttached, disabled, () =>
            runToolAction(toolKey, () =>
              isAttached
                ? detachTool(conversationId, tool.name)
                : attachTool(conversationId, tool.provider.provider_id, tool.provider.tool_name)
            )
          )
        );
        row.appendChild(text);
        row.appendChild(rowActs);
        card.appendChild(row);
      }
    }

    toolsList.appendChild(card);
  }

  // unavailable providers shown honestly with their error
  for (const provider of unavailable) {
    const div = document.createElement("div");
    div.className = "unavail";
    const name = document.createElement("div");
    name.className = "pname";
    name.textContent = provider.display_name || providerLabel(provider.provider_id);
    const u = document.createElement("div");
    u.className = "u";
    u.textContent = "unavailable";
    div.appendChild(name);
    div.appendChild(u);
    if (provider.error) {
      const e = document.createElement("div");
      e.className = "e";
      e.textContent = provider.error;
      div.appendChild(e);
    }
    toolsList.appendChild(div);
  }
}

function openTools() {
  overlayEl.classList.add("open");
  toolsEl.classList.add("open");
  loadToolsPanel();
}

function closeTools() {
  overlayEl.classList.remove("open");
  toolsEl.classList.remove("open");
}

toolsBtn.addEventListener("click", openTools);

// Approval mode toggle.
modeManualBtn.addEventListener("click", async () => {
  approvalMode = "manual";
  if (conversationId) await setToolApprovalMode(conversationId, "manual");
  renderToolsList();
});
modeAutoBtn.addEventListener("click", async () => {
  approvalMode = "auto_approve_attached";
  if (conversationId) await setToolApprovalMode(conversationId, "auto_approve_attached");
  renderToolsList();
});

// Boot: confirm the server is up, make a conversation, set its model.
async function boot() {
  try {
    await health();
    statusEl.textContent = "connected";
  } catch {
    statusEl.textContent = "cannot reach windie api — is `windie api` running?";
    sendEl.disabled = true;
    return;
  }

  // For v1 we always create a fresh conversation on load.
  conversationId = await createConversation();

  // Load the gateway's models, then default to the first so sends work.
  await loadModels();
  if (allModels.length > 0) {
    await pickModel(allModels[0]);
  }

  // Show the current attached-tools count on the TOOLS button.
  loadToolsPanel();
}

// ===========================================================================
// Full-fidelity rendering (mirrors the inspector).
//
// Rule: the live stream is an EPHEMERAL preview. The database is the source of
// truth. On any `*_saved` event we reload the conversation from the report and
// re-render the persisted messages, so saved content (including real tool
// outputs) is exact.
// ===========================================================================

// Extract the text of one message from its parts (fallback to content).
function messageText(message) {
  const part = (message.parts || []).find((p) => p.type === "text");
  return part ? part.text : message.content || "";
}

// Render one persisted message node by role/metadata.
function renderPersistedMessage(message) {
  const role = message.role;
  const metadata = message.metadata || {};

  // Reasoning lane (assistant thinking).
  if (metadata.reasoning) {
    const details = document.createElement("details");
    details.className = "msg assistant reasoning";
    const summary = document.createElement("summary");
    summary.textContent = "reasoning";
    const body = document.createElement("div");
    body.className = "reasoning-body";
    body.textContent = metadata.reasoning;
    details.appendChild(summary);
    details.appendChild(body);
    messagesEl.appendChild(details);
  }

  // Tool result message: mono block of real output text.
  if (role === "tool") {
    const div = document.createElement("div");
    div.className = "msg tool";
    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = metadata.tool_call_id ? `tool · ${metadata.tool_call_id}` : "tool";
    const body = document.createElement("pre");
    body.className = "args";
    body.textContent = messageText(message);
    div.appendChild(meta);
    div.appendChild(body);
    messagesEl.appendChild(div);
    return;
  }

  // User / assistant / system text bubble.
  const label = role === "user" ? "you" : role;
  const body = addMessage(role === "user" ? "user" : "assistant", label);
  body.textContent = messageText(message);

  // Tool-call lane for assistant messages that requested tools.
  if (metadata.tool_calls && metadata.tool_calls.length > 0) {
    for (const call of metadata.tool_calls) {
      const chip = document.createElement("div");
      chip.className = "msg assistant toolcall";
      const name = call.function?.name || "?";
      const args = call.function?.arguments || "";
      chip.textContent = `tool: ${name}\n${args}`;
      messagesEl.appendChild(chip);
    }
  }
}

// Reload the conversation from the report and re-render persisted messages.
// This is the single source-of-truth render used on boot and on every save.
//
// We fetch the report UNSCOPED (no head). With no head, the report's `path`
// and `model_context` are empty by design (they are head-dependent), so the
// authoritative full content is `report.messages` — the whole tree in order.
// For our append-only linear conversation that order IS the conversation.
async function renderConversation() {
  if (!conversationId) return;
  const report = await getConversation(conversationId);

  messagesEl.innerHTML = "";
  const nodes = report.messages || [];
  for (const message of nodes) {
    if (!message.id) continue;
    renderPersistedMessage(message);
  }

  // Advance the local head to the latest message so the next send chains on it.
  const last = nodes[nodes.length - 1];
  if (last && last.id) headMessageId = last.id;

  messagesEl.scrollTop = messagesEl.scrollHeight;
}

// --- ephemeral streaming preview (replaced by persisted render on save) ---

let pendingEl = null;          // the ephemeral streaming bubble
let pendingReasoningEl = null; // ephemeral reasoning block
let pendingToolChips = [];     // ephemeral tool-call chips

function clearPending() {
  for (const el of [pendingEl, pendingReasoningEl, ...pendingToolChips]) {
    if (el && el.parentNode) el.parentNode.removeChild(el);
  }
  pendingEl = null;
  pendingReasoningEl = null;
  pendingToolChips = [];
}

function pendingTextBody() {
  if (!pendingEl) {
    pendingEl = addMessage("assistant", "assistant · streaming");
  }
  return pendingEl;
}

function pendingReasoningBody() {
  if (!pendingReasoningEl) {
    const details = document.createElement("details");
    details.className = "msg assistant reasoning";
    const summary = document.createElement("summary");
    summary.textContent = "reasoning";
    const body = document.createElement("div");
    body.className = "reasoning-body";
    details.appendChild(summary);
    details.appendChild(body);
    messagesEl.appendChild(details);
    pendingReasoningEl = body;
  }
  return pendingReasoningEl;
}

// Render the session's pending approval cards from the server's authoritative
// list. Replaces any existing approval cards so stale/decided calls disappear.
function renderApprovalCards(sessionId, approvals) {
  // Drop any previously rendered approval cards first.
  for (const el of messagesEl.querySelectorAll(".approval")) {
    el.parentNode.removeChild(el);
  }
  for (const approval of approvals) {
    addApprovalCard(sessionId, approval);
  }
}

// One approval card with approve/deny buttons for a pending tool call.
// `approval` is the backend's authoritative pending request:
// { tool_call_id, tool_name, arguments, reason, assistant_message_id, ... }.
function addApprovalCard(sessionId, approval) {
  const card = document.createElement("div");
  card.className = "msg approval";
  card.dataset.toolCallId = approval.tool_call_id;

  const label = document.createElement("div");
  label.className = "meta";
  label.textContent = `approve tool: ${approval.tool_name || "?"}`;

  const args = document.createElement("pre");
  args.className = "args";
  args.textContent = approval.arguments || "";

  const approveBtn = document.createElement("button");
  approveBtn.textContent = "approve";
  const denyBtn = document.createElement("button");
  denyBtn.textContent = "deny";
  denyBtn.className = "deny";

  const settle = async (fn) => {
    approveBtn.disabled = true;
    denyBtn.disabled = true;
    try {
      await fn(sessionId, approval.tool_call_id);
      // Remove just this card; the resumed session will stream/reload the rest.
      card.parentNode.removeChild(card);
    } catch (error) {
      label.textContent = `error: ${error.message}`;
      approveBtn.disabled = false;
      denyBtn.disabled = false;
    }
  };
  approveBtn.addEventListener("click", () => settle(approveSessionTool));
  denyBtn.addEventListener("click", () => settle(denySessionTool));

  card.appendChild(label);
  card.appendChild(args);
  card.appendChild(approveBtn);
  card.appendChild(denyBtn);
  messagesEl.appendChild(card);
  messagesEl.scrollTop = messagesEl.scrollHeight;
}

// Send: save message → start session → stream deltas, reload on save.
async function send() {
  const text = inputEl.value.trim();
  if (!text) return;
  inputEl.value = "";
  sendEl.disabled = true;

  try {
    // 1. save the user message; it becomes the new head
    headMessageId = await insertUserMessage(conversationId, text, headMessageId);
    // show the persisted user message immediately
    await renderConversation();

    // 2. start a session from that head
    const sessionId = await createSession(conversationId, headMessageId);

    // 3. stream events; deltas are ephemeral, saves trigger a reload
    const { apiToken } = await import("./api.js");

    await streamSessionEvents(sessionId, async ({ data }) => {
      if (data.type === "assistant_delta") {
        pendingTextBody().textContent += data.text;
      } else if (data.type === "reasoning_delta") {
        pendingReasoningBody().textContent += data.text;
      } else if (data.type === "waiting_for_approval") {
        // Read the authoritative pending list from the backend (store + policy)
        // instead of deriving it from streamed deltas. This prevents stale or
        // already-decided calls from re-rendering.
        const body = await listSessionApprovals(sessionId);
        renderApprovalCards(sessionId, body.approvals || []);
      } else if (data.type === "assistant_message_saved" || data.type === "tool_result_saved") {
        // A message landed in the store: drop the ephemeral preview and
        // re-render from the persisted report (shows real tool outputs).
        clearPending();
        await renderConversation();
      } else if (data.type === "completed") {
        clearPending();
        await renderConversation();
      } else if (data.type === "failed") {
        clearPending();
        addMessage("assistant", "assistant").textContent = `[error] ${data.error}`;
      }
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }, { apiBase: API_BASE, token: apiToken() });
  } catch (error) {
    clearPending();
    addMessage("assistant", "assistant").textContent = `[error] ${error.message}`;
  } finally {
    sendEl.disabled = false;
    inputEl.focus();
  }
}

sendEl.addEventListener("click", send);
inputEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter") send();
});

boot();
