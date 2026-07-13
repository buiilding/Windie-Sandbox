const API_BASE = process.env.REACT_APP_WINDIE_API_URL || "http://127.0.0.1:8787";
const API_TOKEN_STORAGE_KEY = "windie_api_token";
export const DEFAULT_MODEL = "openai/gpt-4o-mini";

function apiToken() {
  const params = new URLSearchParams(window.location.search);
  const tokenFromUrl = params.get("windie_token");
  if (tokenFromUrl) {
    window.localStorage.setItem(API_TOKEN_STORAGE_KEY, tokenFromUrl);
    return tokenFromUrl;
  }

  return (
    process.env.REACT_APP_WINDIE_API_TOKEN ||
    window.localStorage.getItem(API_TOKEN_STORAGE_KEY) ||
    ""
  );
}

function parseApiBody(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return { error: text };
  }
}

export async function apiRequest(path, options = {}) {
  const token = apiToken();
  const { headers: optionHeaders = {}, ...fetchOptions } = options;
  const response = await fetch(`${API_BASE}${path}`, {
    ...fetchOptions,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { "X-Windie-Api-Token": token } : {}),
      ...optionHeaders,
    },
  });

  const text = await response.text();
  const body = parseApiBody(text);

  if (!response.ok) {
    if (response.status === 401) {
      throw new Error(
        "Windie API token is missing or invalid. Start windie api, then open the inspector with ?windie_token=<printed token>."
      );
    }
    throw new Error(body?.error || `Windie API request failed: ${response.status}`);
  }

  return body;
}

export async function fetchImageAsset(conversationId, assetId) {
  const token = apiToken();
  const response = await fetch(
    `${API_BASE}/api/conversations/${encodeURIComponent(conversationId)}/images/${encodeURIComponent(assetId)}`,
    {
      headers: {
        ...(token ? { "X-Windie-Api-Token": token } : {}),
      },
    }
  );

  if (!response.ok) {
    const text = await response.text();
    const body = parseApiBody(text);
    throw new Error(body?.error || `Windie image request failed: ${response.status}`);
  }

  return response.blob();
}

export async function listModels() {
  const body = await apiRequest("/api/models");
  return (body.models || []).map((model) => ({
    id: model.id,
    label: model.id,
    contextLength: model.context_length ?? null,
    maxInputTokens: model.max_input_tokens ?? null,
    maxOutputTokens: model.max_output_tokens ?? null,
  }));
}

export async function countConversationInputTokens(conversationId, model = null) {
  const body = await apiRequest(
    `/api/conversations/${encodeURIComponent(conversationId)}/input-tokens`,
    {
      method: "POST",
      body: JSON.stringify({ model: model || null }),
    }
  );

  return {
    inputTokens: body?.input_tokens ?? null,
    totalTokens: body?.total_tokens ?? null,
    model: body?.model ?? null,
    source: body?.source || null,
    raw: body?.raw || null,
  };
}

export async function fetchModelParameters(model) {
  return apiRequest(`/api/model-parameters?model=${encodeURIComponent(model)}`);
}

function parseSseBlock(block) {
  const lines = block.split(/\r?\n/);
  let event = "message";
  const data = [];

  for (const line of lines) {
    if (line.startsWith("event:")) {
      event = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      data.push(line.slice("data:".length).trimStart());
    }
  }

  if (data.length === 0) return null;

  return {
    event,
    data: JSON.parse(data.join("\n")),
  };
}

async function streamSse(path, fallbackError, onEvent, options = {}) {
  const token = apiToken();
  const response = await fetch(`${API_BASE}${path}`, {
    headers: {
      ...(token ? { "X-Windie-Api-Token": token } : {}),
    },
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

export async function createRun(conversationId, body = {}) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/runs`, {
    method: "POST",
    body: JSON.stringify({
      head_message_id: body.headMessageId || null,
      model: body.model || null,
      reasoning: body.reasoning || null,
    }),
  });
}

export async function listRuns() {
  const body = await apiRequest("/api/runs");
  return body.runs || [];
}

export async function getRun(runId) {
  return apiRequest(`/api/runs/${encodeURIComponent(runId)}`);
}

export async function streamRunEvents(runId, afterEventId, onEvent, options = {}) {
  const cursor =
    afterEventId == null ? "" : `?after=${encodeURIComponent(String(afterEventId))}`;
  return streamSse(
    `/api/runs/${encodeURIComponent(runId)}/events${cursor}`,
    "Windie run stream failed",
    onEvent,
    options
  );
}

export async function stopRun(runId) {
  return apiRequest(`/api/runs/${encodeURIComponent(runId)}/stop`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function approveRunTool(runId, toolCallId) {
  return apiRequest(
    `/api/runs/${encodeURIComponent(runId)}/approvals/${encodeURIComponent(toolCallId)}/approve`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
}

export async function denyRunTool(runId, toolCallId) {
  return apiRequest(
    `/api/runs/${encodeURIComponent(runId)}/approvals/${encodeURIComponent(toolCallId)}/deny`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
}

export async function setConversationModel(conversationId, model) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/model`, {
    method: "PATCH",
    body: JSON.stringify({ model }),
  });
}

export async function setConversationReasoning(conversationId, effort) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/reasoning`, {
    method: "PATCH",
    body: JSON.stringify({ effort: effort || null }),
  });
}

export function conversationSummaryFromApi(summary) {
  return {
    id: summary.id,
    name: summary.title || `conversation ${summary.id.slice(0, 8)}`,
    model: summary.model || DEFAULT_MODEL,
    systemPrompt: "",
    toolApprovalMode: "manual",
    rootId: null,
    nodes: {},
    activePath: [],
    updatedAt: new Date().toISOString(),
    tags: [],
    messageCount: summary.message_count || 0,
    toolSchemas: [],
  };
}

export function toolCatalogFromApi(body) {
  return (body.tools || []).map(toolSchemaFromApi);
}

export function toolProviderStatusesFromApi(body) {
  return (body.providers || []).map((provider) => ({
    providerId: provider.provider_id,
    displayName: provider.display_name || provider.provider_id,
    available: Boolean(provider.available),
    toolCount: provider.tool_count ?? 0,
    error: provider.error || null,
  }));
}

export function conversationFromInspection(report, fallback) {
  const nodes = {};

  for (const message of report.messages || []) {
    if (!message.id) continue;
    nodes[message.id] = {
      id: message.id,
      parentId: message.parent_message_id || null,
      childrenIds: [],
      message: messageFromApi(message, report.model, report.conversation_id),
    };
  }

  for (const node of Object.values(nodes)) {
    if (node.parentId && nodes[node.parentId]) {
      nodes[node.parentId].childrenIds.push(node.id);
    }
  }

  const activePath = (report.active_path || [])
    .map((message) => message.id)
    .filter((id) => id && nodes[id]);
  const rootIds = Object.values(nodes)
    .filter((node) => node.parentId === null)
    .map((node) => node.id);
  const rootId =
    activePath[0] ||
    rootIds[0] ||
    null;

  return {
    ...(fallback || {}),
    id: report.conversation_id,
    name: fallback?.name || `conversation ${report.conversation_id.slice(0, 8)}`,
    model: report.model,
    reasoning: report.reasoning || null,
    systemPrompt: report.system_prompt || "",
    toolApprovalMode: report.tool_approval_mode || "manual",
    rootId,
    rootIds,
    nodes,
    activePath,
    updatedAt: new Date().toISOString(),
    tags: fallback?.tags || [],
    messageCount: Object.keys(nodes).length,
    toolSchemas: (report.tool_schemas || []).map(toolSchemaFromApi),
    modelContext: report.model_context || [],
    latestCompaction: report.latest_compaction || null,
  };
}

function messageFromApi(message, model, conversationId) {
  const parts = partsFromApi(message, conversationId);
  return {
    role: message.role,
    parts,
    metadata: metadataFromApi(message.metadata),
    model: message.role === "assistant" ? model : undefined,
    timestamp: new Date().toISOString(),
  };
}

function partsFromApi(message, conversationId) {
  if (message.parts?.length) {
    return message.parts.map((part) => {
      if (part.type === "text") {
        return { type: "text", text: part.text || "" };
      }
      return {
        type: "image",
        alt: `${part.asset_id || "image"} · ${part.mime_type || "image"} · ${part.byte_count || 0}b`,
        assetId: part.asset_id,
        conversationId,
        mimeType: part.mime_type,
        byteCount: part.byte_count,
      };
    });
  }

  return [{ type: "text", text: message.content || "" }];
}

function metadataFromApi(metadata) {
  if (!metadata) return null;

  return {
    toolCalls: (metadata.tool_calls || []).map((call) => ({
      id: call.id,
      name: call.function?.name || "",
      arguments: parseJson(call.function?.arguments || "{}"),
      status: "received",
    })),
    toolCallId: metadata.tool_call_id || null,
    reasoning: metadata.reasoning || undefined,
    refusal: metadata.refusal
      ? { category: "provider_refusal", reason: metadata.refusal }
      : undefined,
    annotations: (metadata.annotations || []).map((annotation) => ({
      label: annotation.url_citation?.title || annotation.type || "annotation",
      note: annotation.url_citation?.url || annotation.url_citation?.title || "",
    })),
    audio: metadata.audio
      ? {
          source: metadata.audio.id,
          durationSec: 0,
          speakers: 1,
          transcriptTokens: metadata.audio.transcript?.split(/\s+/).filter(Boolean).length || 0,
        }
      : undefined,
    usage: metadata.usage
      ? {
          inputTokens: metadata.usage.input_tokens ?? null,
          outputTokens: metadata.usage.output_tokens ?? null,
          totalTokens: metadata.usage.total_tokens ?? null,
          raw: metadata.usage.raw || null,
        }
      : undefined,
  };
}

function parseJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    return { raw: value };
  }
}

function toolSchemaFromApi(schema) {
  return {
    name: schema.name,
    description: schema.description,
    parameters: schema.parameters,
    providerId: schema.provider?.provider_id,
    providerToolName: schema.provider?.tool_name,
    providerKind: schema.provider?.kind,
  };
}
