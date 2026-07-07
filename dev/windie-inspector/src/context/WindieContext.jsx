import { createContext, useContext, useMemo, useState, useCallback, useEffect, useRef } from "react";
import { toast } from "sonner";
import {
  apiRequest,
  countConversationInputTokens,
  conversationFromInspection,
  conversationSummaryFromApi,
  listModels,
  toolCatalogFromApi,
} from "@/lib/windieApi";

const WindieCtx = createContext(null);

function fileToBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result || "");
      resolve(result.includes(",") ? result.split(",")[1] : result);
    };
    reader.onerror = () => reject(reader.error || new Error("failed to read image"));
    reader.readAsDataURL(file);
  });
}

async function messagePartsForSend(text, attachments = []) {
  const parts = [];
  if (text.trim()) {
    parts.push({ type: "text", text });
  }

  for (const attachment of attachments) {
    if (attachment.source === "path" && attachment.path) {
      parts.push({ type: "image", path: attachment.path });
    }
    if (attachment.source === "clipboard" && attachment.file) {
      parts.push({
        type: "image_data",
        mime_type: attachment.file.type || "image/png",
        data: await fileToBase64(attachment.file),
      });
    }
  }

  return parts;
}

function tokenCountKey(conversationId, modelId) {
  return `${conversationId || ""}::${modelId || ""}`;
}

function pathNodesForConversation(conversation) {
  if (!conversation) return [];
  return conversation.activePath.map((id) => conversation.nodes[id]).filter(Boolean);
}

function latestAssistantUsageNode(nodes) {
  return [...nodes]
    .reverse()
    .find(
      (node) =>
        node.message.role === "assistant" &&
        node.message.metadata?.usage?.totalTokens != null
    );
}

function stableJson(value) {
  return JSON.stringify(value);
}

function contextSignatureParts(conversation, modelId) {
  if (!conversation) {
    return {
      pathSignature: "",
      setupSignature: "",
      fullSignature: "",
    };
  }

  const pathNodes = pathNodesForConversation(conversation);
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
  const pathSignature = stableJson(path);
  const setupSignature = stableJson(setup);

  return {
    pathSignature,
    setupSignature,
    fullSignature: stableJson({ setup, path }),
  };
}

export function WindieProvider({ children }) {
  const [conversations, setConversations] = useState([]);
  const [activeConvId, setActiveConvId] = useState(null);
  const [selectedNodeId, setSelectedNodeId] = useState(null);
  const [theme, setTheme] = useState("dark");
  const [treeOverlayOpen, setTreeOverlayOpen] = useState(false);
  const [contextPreviewOpen, setContextPreviewOpen] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [modelOverride, setModelOverride] = useState(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [apiError, setApiError] = useState(null);
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [approvals, setApprovals] = useState([]);
  const [availableToolSchemas, setAvailableToolSchemas] = useState([]);
  const [models, setModels] = useState([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(null);
  const [inputTokenCounts, setInputTokenCounts] = useState({});
  const previousContextSignaturesRef = useRef(null);

  useEffect(() => {
    const root = document.documentElement;
    if (theme === "dark") root.classList.add("dark");
    else root.classList.remove("dark");
  }, [theme]);

  const refreshConversations = useCallback(async () => {
    const body = await apiRequest("/api/conversations");
    const summaries = body.conversations.map(conversationSummaryFromApi);

    setConversations((prev) =>
      summaries.map((summary) => {
        const existing = prev.find((conv) => conv.id === summary.id);
        return existing ? { ...summary, ...existing, messageCount: summary.messageCount } : summary;
      })
    );

    return summaries;
  }, []);

  const refreshGateway = useCallback(async () => {
    const body = await apiRequest("/api/status");
    setGatewayRunning(Boolean(body.gateway_running));
    return body.gateway_running;
  }, []);

  const refreshModels = useCallback(async () => {
    setModelsLoading(true);
    try {
      const nextModels = await listModels();
      setModels(nextModels);
      setModelsError(null);
      return nextModels;
    } catch (error) {
      setModels([]);
      setModelsError(error.message);
      throw error;
    } finally {
      setModelsLoading(false);
    }
  }, []);

  const refreshAvailableTools = useCallback(async () => {
    const body = await apiRequest("/api/tools");
    const tools = toolCatalogFromApi(body);
    setAvailableToolSchemas(tools);
    return tools;
  }, []);

  const loadConversation = useCallback(async (convId, options = {}) => {
    if (!convId) return null;
    const query = modelOverride ? `?model=${encodeURIComponent(modelOverride)}` : "";
    const [report, approvalBody] = await Promise.all([
      apiRequest(`/api/conversations/${convId}${query}`),
      apiRequest(`/api/conversations/${convId}/approvals`),
    ]);
    let loaded = null;

    setConversations((prev) => {
      const fallback = prev.find((conv) => conv.id === convId);
      loaded = conversationFromInspection(report, fallback);
      const exists = prev.some((conv) => conv.id === convId);
      if (!exists) return [loaded, ...prev];
      return prev.map((conv) => (conv.id === convId ? loaded : conv));
    });

    if (options.selectLast !== false) {
      const last = loaded?.activePath?.[loaded.activePath.length - 1] || loaded?.rootId || null;
      setSelectedNodeId((current) => (current && loaded?.nodes[current] ? current : last));
    }
    setApprovals(approvalBody.approvals || []);

    return loaded;
  }, [modelOverride]);

  useEffect(() => {
    let cancelled = false;
    refreshConversations()
      .then((summaries) => {
        if (cancelled) return;
        setApiError(null);
        setActiveConvId((current) => current || summaries[0]?.id || null);
      })
      .catch((error) => {
        if (!cancelled) setApiError(error.message);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshConversations]);

  useEffect(() => {
    refreshGateway().catch((error) => setApiError(error.message));
  }, [refreshGateway]);

  useEffect(() => {
    refreshModels().catch(() => {});
  }, [refreshModels]);

  useEffect(() => {
    refreshAvailableTools().catch((error) => setApiError(error.message));
  }, [refreshAvailableTools]);

  useEffect(() => {
    if (!activeConvId) return;
    let cancelled = false;
    loadConversation(activeConvId)
      .then(() => {
        if (!cancelled) setApiError(null);
      })
      .catch((error) => {
        if (!cancelled) setApiError(error.message);
      });
    return () => {
      cancelled = true;
    };
  }, [activeConvId, loadConversation]);

  const activeConv = useMemo(
    () => conversations.find((c) => c.id === activeConvId) || null,
    [conversations, activeConvId]
  );

  const activePathNodes = useMemo(() => {
    return pathNodesForConversation(activeConv);
  }, [activeConv]);

  const activeModelId = useMemo(
    () => modelOverride || activeConv?.model || null,
    [activeConv?.model, modelOverride]
  );

  const activeContextSignatures = useMemo(
    () => contextSignatureParts(activeConv, activeModelId),
    [activeConv, activeModelId]
  );

  const refreshInputTokenCount = useCallback(
    async (conversation, modelId, signature) => {
      if (!conversation?.id) return null;

      const countKey = tokenCountKey(conversation.id, modelId);

      try {
        const count = await countConversationInputTokens(conversation.id, modelId);
        setInputTokenCounts((prev) => ({
          ...prev,
          [countKey]: {
            ...count,
            source: count.source || "prequery_input",
            signature,
            measuredAt: Date.now(),
          },
        }));
        setApiError(null);
        return count;
      } catch (error) {
        setApiError(error.message);
        setInputTokenCounts((prev) => ({
          ...prev,
          [countKey]: {
            inputTokens: null,
            totalTokens: null,
            model: modelId,
            raw: null,
            source: "prequery_input",
            signature,
            measuredAt: Date.now(),
          },
        }));
        return null;
      }
    },
    []
  );

  useEffect(() => {
    if (!activeConv || !activeContextSignatures.fullSignature) return;

    const previous = previousContextSignaturesRef.current;
    const pathChanged =
      !previous || previous.pathSignature !== activeContextSignatures.pathSignature;
    const setupChanged =
      !previous || previous.setupSignature !== activeContextSignatures.setupSignature;
    const latestNode = activePathNodes[activePathNodes.length - 1] || null;
    const latestNodeIsCompletedAssistant =
      latestNode?.message.role === "assistant" &&
      latestNode.message.metadata?.usage?.totalTokens != null;
    previousContextSignaturesRef.current = activeContextSignatures;

    if (latestNodeIsCompletedAssistant && (!previous || (pathChanged && !setupChanged))) {
      return;
    }

    refreshInputTokenCount(activeConv, activeModelId, activeContextSignatures.fullSignature);
  }, [
    activeConv,
    activeModelId,
    activePathNodes,
    activeContextSignatures,
    refreshInputTokenCount,
  ]);

  const tokenMeter = useMemo(() => {
    const selectedModel = models.find((model) => model.id === activeModelId);
    const maxTokens = selectedModel?.contextLength ?? selectedModel?.maxInputTokens ?? null;
    const latestUsageNode = latestAssistantUsageNode(activePathNodes);
    const latestNode = activePathNodes[activePathNodes.length - 1] || null;
    const latestNodeIsCompletedAssistant =
      latestNode?.message.role === "assistant" &&
      latestNode.message.metadata?.usage?.totalTokens != null;
    const inputCount = inputTokenCounts[tokenCountKey(activeConv?.id, activeModelId)] || null;
    const matchingInputCount =
      inputCount?.signature === activeContextSignatures.fullSignature ? inputCount : null;

    return {
      used:
        matchingInputCount?.inputTokens ??
        (latestNodeIsCompletedAssistant
          ? latestUsageNode?.message.metadata?.usage?.totalTokens
          : null) ??
        null,
      max: maxTokens,
      model: activeModelId,
      measuredModel: latestUsageNode?.message.model || null,
      source:
        matchingInputCount?.source ||
        (latestNodeIsCompletedAssistant ? "assistant_total" : null),
    };
  }, [
    activeConv?.id,
    activeContextSignatures.fullSignature,
    activeModelId,
    activePathNodes,
    inputTokenCounts,
    models,
  ]);

  const runMutation = useCallback(
    async (operation, options = {}) => {
      try {
        const result = await operation();
        setApiError(null);
        if (options.refreshList) await refreshConversations();
        if (options.reload !== false && activeConvId) await loadConversation(options.convId || activeConvId);
        return result;
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
        throw error;
      }
    },
    [activeConvId, loadConversation, refreshConversations]
  );

  const createConversation = useCallback(async () => {
    const body = await runMutation(
      () => apiRequest("/api/conversations", { method: "POST" }),
      { reload: false, refreshList: true }
    );
    setActiveConvId(body.conversation_id);
    setSelectedNodeId(null);
    await loadConversation(body.conversation_id);
    return body.conversation_id;
  }, [loadConversation, runMutation]);

  const renameConversation = useCallback(() => {
    toast.message("rename is not a Windie primitive yet");
  }, []);

  const deleteConversation = useCallback(
    async (convId) => {
      await runMutation(
        () => apiRequest(`/api/conversations/${convId}`, { method: "DELETE" }),
        { reload: false, refreshList: false }
      );
      const summaries = await refreshConversations();
      const nextId = summaries.find((conv) => conv.id !== convId)?.id || null;
      setActiveConvId(nextId);
      setSelectedNodeId(null);
      if (nextId) await loadConversation(nextId);
    },
    [loadConversation, refreshConversations, runMutation]
  );

  const setSystemPrompt = useCallback(
    (convId, text) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/system-prompt`, {
          method: "PATCH",
          body: JSON.stringify({ text }),
        })
      ),
    [runMutation]
  );

  const setToolApprovalMode = useCallback(
    (convId, mode) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tool-approval-mode`, {
          method: "PATCH",
          body: JSON.stringify({ mode }),
        })
      ),
    [runMutation]
  );

  const addToolSchema = useCallback(
    (convId, toolSchema) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tools`, {
          method: "POST",
          body: JSON.stringify({
            provider_id: toolSchema.providerId,
            tool_name: toolSchema.providerToolName,
          }),
        })
      ),
    [runMutation]
  );

  const addToolSchemas = useCallback(
    (convId, toolSchemas) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tools/batch`, {
          method: "POST",
          body: JSON.stringify({
            tools: toolSchemas.map((toolSchema) => ({
              provider_id: toolSchema.providerId,
              tool_name: toolSchema.providerToolName,
            })),
          }),
        })
      ),
    [runMutation]
  );

  const removeToolSchema = useCallback(
    (convId, name) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tools/${encodeURIComponent(name)}`, {
          method: "DELETE",
        })
      ),
    [runMutation]
  );

  const removeToolSchemas = useCallback(
    (convId, names) =>
      runMutation(async () => {
        for (const name of names) {
          await apiRequest(`/api/conversations/${convId}/tools/${encodeURIComponent(name)}`, {
            method: "DELETE",
          });
        }
      }),
    [runMutation]
  );

  const setActivePathToLeaf = useCallback(
    (convId, leafId) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/activate`, {
          method: "POST",
          body: JSON.stringify({ message_id: leafId }),
        })
      ),
    [runMutation]
  );

  const setActivePath = useCallback(
    (convId, path) => {
      const leafId = path[path.length - 1];
      if (!leafId) return Promise.resolve();
      return setActivePathToLeaf(convId, leafId);
    },
    [setActivePathToLeaf]
  );

  const truncateAfter = useCallback(
    (convId, nodeId) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/truncate`, {
          method: "POST",
          body: JSON.stringify({ message_id: nodeId }),
        })
      ),
    [runMutation]
  );

  const removeMessage = useCallback(
    (convId, nodeId) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, {
          method: "DELETE",
        })
      ),
    [runMutation]
  );

  const editMessage = useCallback(
    (convId, nodeId, newText) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, {
          method: "PATCH",
          body: JSON.stringify({ text: newText }),
        })
      ),
    [runMutation]
  );

  const forkFromMessage = useCallback(
    async (convId, nodeId) => {
      const body = await runMutation(
        () =>
          apiRequest(`/api/conversations/${convId}/fork`, {
            method: "POST",
            body: JSON.stringify({ message_id: nodeId }),
          }),
        { reload: false, refreshList: true }
      );
      setActiveConvId(body.conversation_id);
      setSelectedNodeId(null);
      await loadConversation(body.conversation_id);
      return body.conversation_id;
    },
    [loadConversation, runMutation]
  );

  const sendMessage = useCallback(
    async (convId, text, options = {}) => {
      const attachments = options.attachments || [];
      if ((!text.trim() && attachments.length === 0) || streaming) return;
      setStreaming(true);
      try {
        const parts = await messagePartsForSend(text, attachments);
        await apiRequest(`/api/conversations/${convId}/messages`, {
          method: "POST",
          body: JSON.stringify({ role: "user", parts }),
        });
        await loadConversation(convId);
        await apiRequest(`/api/conversations/${convId}/query`, {
          method: "POST",
          body: JSON.stringify({ model: options.modelOverride || modelOverride }),
        });
        await loadConversation(convId);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      } finally {
        setStreaming(false);
      }
    },
    [loadConversation, modelOverride, streaming]
  );

  const continueConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId || streaming) return;
      setStreaming(true);
      try {
        await apiRequest(`/api/conversations/${convId}/query`, {
          method: "POST",
          body: JSON.stringify({ model: options.modelOverride || modelOverride }),
        });
        await loadConversation(convId);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      } finally {
        setStreaming(false);
      }
    },
    [loadConversation, modelOverride, streaming]
  );

  const startGateway = useCallback(
    () =>
      runMutation(
        async () => {
          const result = await apiRequest("/api/gateway/start", { method: "POST" });
          await refreshGateway();
          await refreshModels().catch(() => {});
          return result;
        },
        { reload: false }
      ),
    [refreshGateway, refreshModels, runMutation]
  );

  const stopGateway = useCallback(
    () =>
      runMutation(
        async () => {
          const result = await apiRequest("/api/gateway/stop", { method: "POST" });
          await refreshGateway();
          await refreshModels().catch(() => {});
          return result;
        },
        { reload: false }
      ),
    [refreshGateway, refreshModels, runMutation]
  );

  const approveToolCall = useCallback(
    (convId, toolCallId) =>
      runMutation(async () => {
        const result = await apiRequest(`/api/conversations/${convId}/approvals/${toolCallId}/approve`, {
          method: "POST",
        });
        await loadConversation(convId);
        await apiRequest(`/api/conversations/${convId}/query`, {
          method: "POST",
          body: JSON.stringify({ model: modelOverride }),
        });
        return result;
      }),
    [loadConversation, modelOverride, runMutation]
  );

  const denyToolCall = useCallback(
    (convId, toolCallId) =>
      runMutation(async () => {
        const result = await apiRequest(`/api/conversations/${convId}/approvals/${toolCallId}/deny`, {
          method: "POST",
        });
        await loadConversation(convId);
        await apiRequest(`/api/conversations/${convId}/query`, {
          method: "POST",
          body: JSON.stringify({ model: modelOverride }),
        });
        return result;
      }),
    [loadConversation, modelOverride, runMutation]
  );

  const value = {
    conversations,
    activeConv,
    activeConvId,
    selectedNodeId,
    activePathNodes,
    theme,
    treeOverlayOpen,
    contextPreviewOpen,
    streaming,
    modelOverride,
    searchQuery,
    models,
    modelsLoading,
    modelsError,
    tokenMeter,
    toolSchemas: activeConv?.toolSchemas || [],
    availableToolSchemas,
    apiError,
    gatewayRunning,
    approvals,
    setActiveConvId,
    setSelectedNodeId,
    setTheme,
    setTreeOverlayOpen,
    setContextPreviewOpen,
    setModelOverride,
    setSearchQuery,
    refreshModels,
    createConversation,
    renameConversation,
    deleteConversation,
    setSystemPrompt,
    setToolApprovalMode,
    addToolSchema,
    addToolSchemas,
    removeToolSchema,
    removeToolSchemas,
    setActivePath,
    setActivePathToLeaf,
    truncateAfter,
    removeMessage,
    editMessage,
    forkFromMessage,
    sendMessage,
    continueConversation,
    startGateway,
    stopGateway,
    refreshGateway,
    approveToolCall,
    denyToolCall,
    refreshConversations,
    loadConversation,
  };

  return <WindieCtx.Provider value={value}>{children}</WindieCtx.Provider>;
}

export function useWindie() {
  const ctx = useContext(WindieCtx);
  if (!ctx) throw new Error("useWindie must be used within WindieProvider");
  return ctx;
}
