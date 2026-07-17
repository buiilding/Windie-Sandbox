export const DEFAULT_MODEL = "openai/gpt-4o-mini";

export function conversationSummaryFromApi(summary) {
  return {
    id: summary.id,
    name: summary.title || `conversation ${summary.id.slice(0, 8)}`,
    model: summary.model || DEFAULT_MODEL,
    systemPrompt: "",
    toolApprovalMode: "manual",
    rootId: null,
    nodes: {},
    selectedPath: [],
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

export function sessionFromApi(session) {
  if (!session) return null;
  return {
    id: session.id,
    conversationId: session.conversation_id,
    startHeadMessageId: session.start_head_message_id || null,
    currentHeadMessageId: session.current_head_message_id || null,
    status: session.status,
    model: session.model,
    reasoning: session.reasoning || null,
    error: session.error || null,
    createdAt: session.created_at,
    updatedAt: session.updated_at,
  };
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

  const selectedPath = (report.path || report.selected_path || [])
    .map((message) => message.id)
    .filter((id) => id && nodes[id]);
  const rootIds = Object.values(nodes)
    .filter((node) => node.parentId === null)
    .map((node) => node.id);
  const rootId = selectedPath[0] || rootIds[0] || null;

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
    selectedPath,
    updatedAt: new Date().toISOString(),
    tags: fallback?.tags || [],
    messageCount: Object.keys(nodes).length,
    toolSchemas: (report.tool_schemas || []).map(toolSchemaFromApi),
    modelContext: report.model_context || [],
    latestCompaction: report.latest_compaction || null,
    paths: (report.paths || []).map((path) => ({
      messageIds: Array.isArray(path.message_ids) ? path.message_ids : [],
      leafMessageId: path.leaf_message_id || null,
      depth: typeof path.depth === "number" ? path.depth : 0,
      leafPreview: path.leaf_preview || "",
    })),
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
