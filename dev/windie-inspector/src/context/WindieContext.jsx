import { createContext, useContext, useMemo, useState, useCallback, useEffect } from "react";
import { toast } from "sonner";
import {
  MODELS,
  apiRequest,
  conversationFromInspection,
  conversationSummaryFromApi,
  toolCatalogFromApi,
} from "@/lib/windieApi";

const WindieCtx = createContext(null);

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
    if (!activeConv) return [];
    return activeConv.activePath.map((id) => activeConv.nodes[id]).filter(Boolean);
  }, [activeConv]);

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

  const addToolSchema = useCallback(
    (convId, toolSchema) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tool-schemas`, {
          method: "POST",
          body: JSON.stringify(toolSchema),
        })
      ),
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
      if (!text.trim() || streaming) return;
      setStreaming(true);
      try {
        await apiRequest(`/api/conversations/${convId}/messages`, {
          method: "POST",
          body: JSON.stringify(
            options.imagePath
              ? {
                  role: "user",
                  parts: [
                    { type: "text", text },
                    { type: "image", path: options.imagePath },
                  ],
                }
              : { role: "user", text }
          ),
        });
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
          return result;
        },
        { reload: false }
      ),
    [refreshGateway, runMutation]
  );

  const stopGateway = useCallback(
    () =>
      runMutation(
        async () => {
          const result = await apiRequest("/api/gateway/stop", { method: "POST" });
          await refreshGateway();
          return result;
        },
        { reload: false }
      ),
    [refreshGateway, runMutation]
  );

  const approveToolCall = useCallback(
    (convId, toolCallId) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/approvals/${toolCallId}/approve`, {
          method: "POST",
        })
      ),
    [runMutation]
  );

  const denyToolCall = useCallback(
    (convId, toolCallId) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/approvals/${toolCallId}/deny`, {
          method: "POST",
        })
      ),
    [runMutation]
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
    models: MODELS,
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
    createConversation,
    renameConversation,
    deleteConversation,
    setSystemPrompt,
    addToolSchema,
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
