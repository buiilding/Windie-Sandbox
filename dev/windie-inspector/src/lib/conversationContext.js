/**
 * Shared conversation-path projections used by rendering and token tracking.
 * These helpers do not decide context; the backend inspection remains
 * authoritative for the actual model-facing context.
 */

export function pathNodesForConversation(conversation) {
  if (!conversation) return [];
  return (conversation.selectedPath || []).map((id) => conversation.nodes[id]).filter(Boolean);
}

function stableJson(value) {
  return JSON.stringify(value);
}

export function contextSignatureParts(conversation, modelId, pathNodesOverride = null) {
  if (!conversation) {
    return { pathSignature: "", setupSignature: "", fullSignature: "" };
  }
  const pathNodes = pathNodesOverride || pathNodesForConversation(conversation);
  const path = pathNodes.map((node) => ({
    id: node.id,
    role: node.message.role,
    parts: node.message.parts || [],
    metadata: {
      toolCalls: node.message.metadata?.toolCalls || [],
      toolCallId: node.message.metadata?.toolCallId || null,
    },
  }));
  const setup = {
    conversationId: conversation.id,
    model: modelId || conversation.model || null,
    systemPrompt: conversation.systemPrompt || "",
    toolSchemas: (conversation.toolSchemas || []).map((tool) => ({
      name: tool.name,
      description: tool.description,
      parameters: tool.parameters,
      providerId: tool.providerId,
      providerToolName: tool.providerToolName,
    })),
    latestCompaction: conversation.latestCompaction || null,
  };
  return {
    pathSignature: stableJson(path),
    setupSignature: stableJson(setup),
    fullSignature: stableJson({ setup, path }),
  };
}
