import { createContext, useContext, useMemo, useState, useCallback, useEffect } from "react";
import { toast } from "sonner";
import {
  apiRequest,
  countConversationInputTokens,
  conversationFromInspection,
  conversationSummaryFromApi,
  fetchModelParameters,
  listModels,
  setConversationModel as setConversationModelApi,
  streamApproveTool,
  streamConversationQuery,
  streamDenyTool,
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
  const [searchQuery, setSearchQuery] = useState("");
  const [apiError, setApiError] = useState(null);
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [approvals, setApprovals] = useState([]);
  const [availableToolSchemas, setAvailableToolSchemas] = useState([]);
  const [models, setModels] = useState([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(null);
  const [inputTokenCounts, setInputTokenCounts] = useState({});
  const [modelParametersById, setModelParametersById] = useState({});
  const [reasoningByConversationId, setReasoningByConversationId] = useState({});
  // Ephemeral live assistant preview from SSE delta events. Display-only: the
  // persisted message that arrives via `assistant_message_saved` is the source
  // of truth and replaces this.
  const [pendingAssistant, setPendingAssistant] = useState(null);

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
        return existing
          ? {
              ...summary,
              ...existing,
              model: summary.model,
              messageCount: summary.messageCount,
            }
          : summary;
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

  const loadModelParameters = useCallback(async (modelId) => {
    if (!modelId) return null;
    const existing = modelParametersById[modelId];
    if (existing?.status === "ready") return existing.data;
    if (existing?.status === "loading") return null;

    setModelParametersById((prev) => ({
      ...prev,
      [modelId]: { status: "loading", data: prev[modelId]?.data || null, error: null },
    }));

    try {
      const data = await fetchModelParameters(modelId);
      setModelParametersById((prev) => ({
        ...prev,
        [modelId]: { status: "ready", data, error: null },
      }));
      return data;
    } catch (error) {
      setModelParametersById((prev) => ({
        ...prev,
        [modelId]: { status: "idle", data: null, error: null },
      }));
      return null;
    }
  }, [modelParametersById]);

  const refreshAvailableTools = useCallback(async () => {
    const body = await apiRequest("/api/tools");
    const tools = toolCatalogFromApi(body);
    setAvailableToolSchemas(tools);
    return tools;
  }, []);

  const loadConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId) return null;
      const [report, approvalBody] = await Promise.all([
        apiRequest(`/api/conversations/${convId}`),
        apiRequest(`/api/conversations/${convId}/approvals`),
      ]);
      const loaded = conversationFromInspection(report, null);

      setConversations((prev) => {
        const fallback = prev.find((conv) => conv.id === convId);
        const loadedWithFallback = conversationFromInspection(report, fallback);
        const exists = prev.some((conv) => conv.id === convId);
        if (!exists) return [loadedWithFallback, ...prev];
        return prev.map((conv) => (conv.id === convId ? loadedWithFallback : conv));
      });

      if (options.selectLast !== false) {
        const last = loaded?.activePath?.[loaded.activePath.length - 1] || loaded?.rootId || null;
        setSelectedNodeId((current) => (current && loaded?.nodes[current] ? current : last));
      }
      setApprovals(approvalBody.approvals || []);

      if (options.countTokens !== false && loaded?.id) {
        const loadedModelId = loaded?.model || null;
        const signature = contextSignatureParts(loaded, loadedModelId).fullSignature;
        const countKey = tokenCountKey(loaded?.id, loadedModelId);
        const requestId = `${Date.now()}-${Math.random().toString(16).slice(2)}`;

        setInputTokenCounts((prev) => ({
          ...prev,
          [countKey]: {
            ...(prev[countKey] || {}),
            latestRequestId: requestId,
          },
        }));

        countConversationInputTokens(loaded.id)
          .then((count) => {
            setInputTokenCounts((prev) => {
              if (prev[countKey]?.latestRequestId !== requestId) return prev;

              return {
                ...prev,
                [countKey]: {
                  ...count,
                  source: count.source || "prequery_input",
                  signature,
                  latestRequestId: requestId,
                  measuredAt: Date.now(),
                },
              };
            });
          })
          .catch((error) => {
            setApiError(error.message);
            setInputTokenCounts((prev) => {
              if (prev[countKey]?.latestRequestId !== requestId) return prev;

              return {
                ...prev,
                [countKey]: {
                  inputTokens: null,
                  totalTokens: null,
                  model: loadedModelId,
                  raw: null,
                  source: "prequery_input",
                  signature,
                  latestRequestId: requestId,
                  measuredAt: Date.now(),
                },
              };
            });
          });
      }

      return loaded;
    },
    []
  );

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
    () => activeConv?.model || null,
    [activeConv?.model]
  );

  const activeContextSignatures = useMemo(
    () => contextSignatureParts(activeConv, activeModelId),
    [activeConv, activeModelId]
  );

  const tokenMeter = useMemo(() => {
    const selectedModel = models.find((model) => model.id === activeModelId);
    const maxTokens = selectedModel?.contextLength ?? selectedModel?.maxInputTokens ?? null;
    const inputCount = inputTokenCounts[tokenCountKey(activeConv?.id, activeModelId)] || null;

    return {
      used: inputCount?.inputTokens ?? null,
      max: maxTokens,
      model: activeModelId,
      measuredModel: inputCount?.model || null,
      source: inputCount?.source || null,
    };
  }, [
    activeConv?.id,
    activeModelId,
    inputTokenCounts,
    models,
  ]);

  useEffect(() => {
    if (activeModelId) {
      loadModelParameters(activeModelId);
    }
  }, [activeModelId, loadModelParameters]);

  const activeModelParameters = useMemo(
    () => modelParametersById[activeModelId] || null,
    [activeModelId, modelParametersById]
  );

  const activeReasoning = useMemo(
    () => reasoningByConversationId[activeConv?.id] || null,
    [activeConv?.id, reasoningByConversationId]
  );

  const setConversationReasoningEffort = useCallback((convId, effort) => {
    if (!convId) return;
    setReasoningByConversationId((prev) => {
      if (!effort) {
        const next = { ...prev };
        delete next[convId];
        return next;
      }

      return {
        ...prev,
        [convId]: { effort },
      };
    });
  }, []);

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

  const setConversationModel = useCallback(
    (convId, model) =>
      runMutation(async () => {
        const result = await setConversationModelApi(convId, model);
        setReasoningByConversationId((prev) => {
          const next = { ...prev };
          delete next[convId];
          return next;
        });
        loadModelParameters(model);
        return result;
      }, {
        convId,
        refreshList: true,
      }),
    [loadModelParameters, runMutation]
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

  const consumeRuntimeStream = useCallback(
    async (convId, stream) => {
      try {
        await stream(async ({ data }) => {
          if (data?.type === "assistant_delta") {
            // Ephemeral live model text. Accumulate into the pending bubble;
            // the persisted message replaces it once saved.
            setPendingAssistant((prev) =>
              prev && prev.convId === convId
                ? { ...prev, text: prev.text + (data.text || "") }
                : { convId, text: data.text || "", reasoning: "", toolCalls: {} }
            );
            return;
          }
          if (data?.type === "reasoning_delta") {
            setPendingAssistant((prev) =>
              prev && prev.convId === convId
                ? { ...prev, reasoning: (prev.reasoning || "") + (data.text || "") }
                : {
                    convId,
                    text: "",
                    reasoning: data.text || "",
                    toolCalls: {},
                  }
            );
            return;
          }
          if (data?.type === "tool_call_delta") {
            setPendingAssistant((prev) => {
              const base =
                prev && prev.convId === convId
                  ? prev
                  : { convId, text: "", reasoning: "", toolCalls: {} };
              const index = String(data.index ?? 0);
              const existing = base.toolCalls?.[index] || {
                id: null,
                name: null,
                argumentsText: "",
              };

              return {
                ...base,
                toolCalls: {
                  ...(base.toolCalls || {}),
                  [index]: {
                    id: data.id || existing.id,
                    name: data.name || existing.name,
                    argumentsText:
                      existing.argumentsText + (data.arguments_delta || ""),
                  },
                },
              };
            });
            return;
          }
          if (
            data?.type === "assistant_message_saved" ||
            data?.type === "tool_result_saved"
          ) {
            await loadConversation(convId);
            // The durable message now renders from the store; drop the
            // ephemeral preview so it is not shown twice.
            setPendingAssistant(null);
          }
          if (data?.type === "query_done") {
            await loadConversation(convId, { countTokens: false });
            setPendingAssistant(null);
          }
        });
      } catch (error) {
        setPendingAssistant(null);
        await loadConversation(convId, { countTokens: false }).catch(() => {});
        throw error;
      }
    },
    [loadConversation]
  );

  const runStreamingQuery = useCallback(
    async (convId) =>
      consumeRuntimeStream(convId, (onEvent) =>
        streamConversationQuery(convId, null, reasoningByConversationId[convId] || null, onEvent)
      ),
    [consumeRuntimeStream, reasoningByConversationId]
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
        await runStreamingQuery(convId);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      } finally {
        setStreaming(false);
      }
    },
    [loadConversation, runStreamingQuery, streaming]
  );

  const continueConversation = useCallback(
    async (convId) => {
      if (!convId || streaming) return;
      setStreaming(true);
      try {
        await runStreamingQuery(convId);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      } finally {
        setStreaming(false);
      }
    },
    [runStreamingQuery, streaming]
  );

  const startGateway = useCallback(
    () =>
      runMutation(
        async () => {
          const result = await apiRequest("/api/gateway/start", { method: "POST" });
          await refreshGateway();
          await refreshModels().catch(() => {});
          if (activeModelId) await loadModelParameters(activeModelId);
          return result;
        },
        { reload: false }
      ),
    [activeModelId, loadModelParameters, refreshGateway, refreshModels, runMutation]
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
      runMutation(
        () =>
          consumeRuntimeStream(convId, (onEvent) =>
            streamApproveTool(convId, toolCallId, onEvent)
          ),
        { reload: false }
      ),
    [consumeRuntimeStream, runMutation]
  );

  const denyToolCall = useCallback(
    (convId, toolCallId) =>
      runMutation(
        () =>
          consumeRuntimeStream(convId, (onEvent) =>
            streamDenyTool(convId, toolCallId, onEvent)
          ),
        { reload: false }
      ),
    [consumeRuntimeStream, runMutation]
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
    pendingAssistant,
    searchQuery,
    models,
    modelsLoading,
    modelsError,
    modelParametersById,
    activeModelParameters,
    activeReasoning,
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
    setSearchQuery,
    refreshModels,
    loadModelParameters,
    createConversation,
    renameConversation,
    deleteConversation,
    setSystemPrompt,
    setConversationModel,
    setConversationReasoningEffort,
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
