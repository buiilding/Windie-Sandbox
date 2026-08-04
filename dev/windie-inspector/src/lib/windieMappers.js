export function conversationSummaryFromApi(summary) {
  return {
    id: summary.id,
    name: summary.title || `conversation ${summary.id.slice(0, 8)}`,
    model: summary.model || null,
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

export function providerInstallationsFromApi(body) {
  const providers = Array.isArray(body) ? body : body.providers || [];
  return providers.map((provider) => ({
    providerId: provider.manifest?.provider_id || "unknown",
    displayName: provider.manifest?.display_name || provider.manifest?.provider_id || "Unknown extension",
    description: provider.manifest?.description || "",
    kind: provider.manifest?.kind || "mcp",
    transport: provider.manifest?.transport || "stdio",
    runtime: provider.manifest?.runtime || "native",
    package: provider.manifest?.package || null,
    launch: provider.manifest?.launch || null,
    platforms: provider.manifest?.platforms || [],
    dependencies: provider.manifest?.dependencies || [],
    secrets: provider.manifest?.secrets || [],
    permissions: provider.manifest?.permissions || [],
    installation: provider.installation
      ? {
          state: provider.installation.state,
          readiness: provider.installation.readiness || null,
          nextAction: provider.installation.next_action || null,
          error: provider.installation.error || null,
          installedAt: provider.installation.installed_at,
          updatedAt: provider.installation.updated_at,
          lastHealthCheckAt: provider.installation.last_health_check_at,
        }
      : null,
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
    queued: Boolean(session.queued),
    queueDepth: session.queue_depth || 0,
    queueId: session.queue_id || null,
    latestEventId: session.latest_event_id ?? null,
    nodeCount: session.node_count ?? 0,
    protectedMessageIds: session.protected_message_ids || [],
    deletionAllowed: session.deletion_allowed !== false,
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

export function upsertConversationMessage(conversation, message, model, updatePath = false) {
  if (!conversation || !message?.id) return conversation;

  const existing = conversation.nodes?.[message.id];
  const node = {
    ...(existing || {}),
    id: message.id,
    parentId: message.parent_message_id || null,
    childrenIds: existing?.childrenIds || [],
    message: messageFromApi(message, model, conversation.id),
  };
  const nodes = { ...(conversation.nodes || {}), [node.id]: node };

  if (node.parentId && nodes[node.parentId]) {
    nodes[node.parentId] = {
      ...nodes[node.parentId],
      childrenIds: Array.from(new Set([
        ...(nodes[node.parentId].childrenIds || []),
        node.id,
      ])),
    };
  }

  const rootIds = node.parentId
    ? (conversation.rootIds || (conversation.rootId ? [conversation.rootId] : []))
    : Array.from(new Set([...(conversation.rootIds || []), node.id]));
  const next = {
    ...conversation,
    nodes,
    rootIds,
    rootId: conversation.rootId || (!node.parentId ? node.id : null),
    messageCount: Object.keys(nodes).length,
  };

  if (updatePath) {
    const path = [];
    const seen = new Set();
    let current = next.nodes[node.id];
    while (current && !seen.has(current.id)) {
      path.push(current.id);
      seen.add(current.id);
      current = current.parentId ? next.nodes[current.parentId] : null;
    }
    next.selectedPath = path.reverse();
  }

  return next;
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
