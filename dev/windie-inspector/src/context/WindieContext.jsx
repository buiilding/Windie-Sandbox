import {
  createContext,
  useContext,
  useMemo,
  useState,
  useCallback,
  useEffect,
  useRef,
} from "react";
import { toast } from "sonner";
import {
  apiRequest,
  countConversationInputTokens,
  fetchModelParameters,
  listModels,
  setConversationModel as setConversationModelApi,
  setConversationReasoning as setConversationReasoningApi,
  listProviderInstallations,
  setupProvider as setupProviderApi,
  enableProvider as enableProviderApi,
  disableProvider as disableProviderApi,
  repairProvider as repairProviderApi,
  uninstallProvider as uninstallProviderApi,
} from "@/lib/windieApi";
import {
  conversationFromInspection,
  conversationSummaryFromApi,
  providerInstallationsFromApi,
  toolCatalogFromApi,
  toolProviderStatusesFromApi,
} from "@/lib/windieMappers";
import { useSessionRuntime } from "@/hooks/useSessionRuntime";

const WindieCtx = createContext(null);

function tokenCountKey(conversationId, modelId) {
  return `${conversationId || ""}::${modelId || ""}`;
}

function isAbortError(error) {
  return error?.name === "AbortError";
}

function pathNodesForConversation(conversation) {
  if (!conversation) return [];
  return (conversation.selectedPath || []).map((id) => conversation.nodes[id]).filter(Boolean);
}

function pathNodesToNode(conversation, nodeId) {
  if (!conversation || !nodeId || !conversation.nodes[nodeId]) {
    return pathNodesForConversation(conversation);
  }
  const reversed = [];
  const seen = new Set();
  let current = conversation.nodes[nodeId];
  while (current && !seen.has(current.id)) {
    reversed.push(current);
    seen.add(current.id);
    current = current.parentId ? conversation.nodes[current.parentId] : null;
  }
  return reversed.reverse();
}

function latestAssistantTotalTokens(pathNodes) {
  for (let index = pathNodes.length - 1; index >= 0; index -= 1) {
    const node = pathNodes[index];
    if (node.message.role !== "assistant") continue;
    const totalTokens = node.message.metadata?.usage?.totalTokens;
    if (totalTokens != null) return totalTokens;
  }
  return null;
}

function stableJson(value) {
  return JSON.stringify(value);
}

function contextSignatureParts(conversation, modelId, pathNodesOverride = null) {
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

export function WindieProvider({ children }) {
  const [conversations, setConversations] = useState([]);
  const [activeConvId, setActiveConvId] = useState(null);
  const [selectedNodeId, setSelectedNodeId] = useState(null);
  const [viewHeadId, setViewHeadId] = useState(null);
  const [theme, setTheme] = useState("dark");
  const [treeOverlayOpen, setTreeOverlayOpen] = useState(false);
  const [contextPreviewOpen, setContextPreviewOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [apiError, setApiError] = useState(null);
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [approvals, setApprovals] = useState([]);
  const [availableToolSchemas, setAvailableToolSchemas] = useState([]);
  const [availableToolsLoading, setAvailableToolsLoading] = useState(false);
  const [toolProviderStatuses, setToolProviderStatuses] = useState([]);
  const [providerInstallations, setProviderInstallations] = useState([]);
  const [providerInstallationsLoading, setProviderInstallationsLoading] = useState(false);
  const [models, setModels] = useState([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(null);
  const [inputTokenCounts, setInputTokenCounts] = useState({});
  const [modelParametersById, setModelParametersById] = useState({});

  // The selection ref is only an async load anchor. The rendered selection
  // remains selectedNodeId; the session runtime owns session selection.
  const selectedNodeRef = useRef(null);
  const loadSeqRef = useRef({});
  const inputTokenSupportRef = useRef({});

  useEffect(() => {
    selectedNodeRef.current = selectedNodeId;
  }, [selectedNodeId]);

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
        return existing ? { ...summary, ...existing, model: summary.model, messageCount: summary.messageCount } : summary;
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

  const loadModelParameters = useCallback(
    async (modelId) => {
      if (!modelId) return null;
      if (!gatewayRunning || modelsLoading || modelsError) return null;
      if (!models.some((model) => model.id === modelId)) return null;
      const existing = modelParametersById[modelId];
      if (existing?.status === "ready") return existing.data;
      if (existing?.status === "loading" || existing?.status === "error") return null;
      setModelParametersById((prev) => ({
        ...prev,
        [modelId]: { status: "loading", data: prev[modelId]?.data || null, error: null },
      }));
      try {
        const data = await fetchModelParameters(modelId);
        setModelParametersById((prev) => ({ ...prev, [modelId]: { status: "ready", data, error: null } }));
        return data;
      } catch (error) {
        setModelParametersById((prev) => ({ ...prev, [modelId]: { status: "error", data: null, error: error.message } }));
        return null;
      }
    },
    [gatewayRunning, modelParametersById, models, modelsError, modelsLoading]
  );

  const refreshAvailableTools = useCallback(async () => {
    setAvailableToolsLoading(true);
    try {
      const body = await apiRequest("/api/tools");
      setAvailableToolSchemas(toolCatalogFromApi(body));
      setToolProviderStatuses(toolProviderStatusesFromApi(body));
      return toolCatalogFromApi(body);
    } finally {
      setAvailableToolsLoading(false);
    }
  }, []);

  const refreshProviderInstallations = useCallback(async () => {
    setProviderInstallationsLoading(true);
    try {
      const nextProviders = providerInstallationsFromApi(await listProviderInstallations());
      setProviderInstallations(nextProviders);
      return nextProviders;
    } finally {
      setProviderInstallationsLoading(false);
    }
  }, []);

  const loadConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId) return null;
      const hasHead = Object.prototype.hasOwnProperty.call(options, "headMessageId");
      const headMessageId = hasHead
        ? options.headMessageId
        : selectedNodeRef.current;
      const q = headMessageId ? `?head_message_id=${encodeURIComponent(headMessageId)}` : "";
      // Latest-wins guard: bump this conversation's load sequence and only apply
      // the result if no newer load started while we were awaiting.
      const seq = (loadSeqRef.current[convId] || 0) + 1;
      loadSeqRef.current[convId] = seq;
      const [report, approvalBody] = await Promise.all([
        apiRequest(`/api/conversations/${convId}${q}`),
        apiRequest(`/api/conversations/${convId}/run-approvals`),
      ]);
      if (loadSeqRef.current[convId] !== seq) {
        // A newer load started; discard this stale response.
        return null;
      }
      const loaded = conversationFromInspection(report, null);
      setConversations((prev) => {
        const fallback = prev.find((conv) => conv.id === convId);
        const withFallback = conversationFromInspection(report, fallback);
        return prev.some((c) => c.id === convId) ? prev.map((c) => (c.id === convId ? withFallback : c)) : [withFallback, ...prev];
      });
      const last = loaded?.selectedPath?.[loaded.selectedPath.length - 1] || loaded?.rootId || null;
      setSelectedNodeId((cur) => (cur && loaded?.nodes?.[cur] ? cur : last));
      setApprovals(approvalBody.approvals || []);

      if (options.countTokens !== false && loaded?.id) {
        const mid = loaded?.model || null;
        const sig = contextSignatureParts(loaded, mid).fullSignature;
        const key = tokenCountKey(loaded?.id, mid);
        if (mid && inputTokenSupportRef.current[mid] === "unsupported") {
          setInputTokenCounts((p) => ({
            ...p,
            [key]: {
              inputTokens: null,
              totalTokens: null,
              model: mid,
              raw: null,
              source: "unsupported",
              signature: sig,
              measuredAt: Date.now(),
            },
          }));
          return loaded;
        }
        const rid = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
        setInputTokenCounts((p) => ({ ...p, [key]: { ...(p[key] || {}), latestRequestId: rid } }));
        countConversationInputTokens(loaded.id, null, headMessageId || null)
          .then((count) => {
            if (mid && count.source === "unsupported") {
              inputTokenSupportRef.current[mid] = "unsupported";
            } else if (mid && count.inputTokens != null) {
              inputTokenSupportRef.current[mid] = "supported";
            }
            setInputTokenCounts((p) => {
              if (p[key]?.latestRequestId !== rid) return p;
              return { ...p, [key]: { ...count, source: count.source || "prequery_input", signature: sig, latestRequestId: rid, measuredAt: Date.now() } };
            });
          })
          .catch(() => {
            setInputTokenCounts((p) => {
              if (p[key]?.latestRequestId !== rid) return p;
              return { ...p, [key]: { inputTokens: null, totalTokens: null, model: mid, raw: null, source: "unavailable", signature: sig, latestRequestId: rid, measuredAt: Date.now() } };
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
      .then((s) => {
        if (cancelled) return;
        setApiError(null);
        setActiveConvId((c) => c || s[0]?.id || null);
      })
      .catch((e) => {
        if (!cancelled) setApiError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshConversations]);

  useEffect(() => {
    refreshGateway().catch((e) => setApiError(e.message));
  }, [refreshGateway]);

  useEffect(() => {
    refreshModels().catch(() => {});
  }, [refreshModels]);

  useEffect(() => {
    refreshAvailableTools().catch((e) => setApiError(e.message));
  }, [refreshAvailableTools]);

  useEffect(() => {
    refreshProviderInstallations().catch((e) => setApiError(e.message));
  }, [refreshProviderInstallations]);

  const activeConv = useMemo(() => conversations.find((c) => c.id === activeConvId) || null, [conversations, activeConvId]);
  const activeModelId = activeConv?.model || null;
  const activeReasoning = activeConv?.reasoning || null;
  const sessionRuntime = useSessionRuntime({
    conversationId: activeConvId,
    conversationModel: activeModelId,
    reasoning: activeReasoning,
    viewHeadId,
    setViewHeadId,
    selectedNodeId,
    setSelectedNodeId,
    setConversations,
    loadConversation,
    setApiError,
  });
  const {
    sessionsById,
    selectedSession,
    selectedSessionId,
    getSelectedSession,
    pendingAssistant,
    streaming,
    refreshSessions,
    selectSession,
    sendMessage,
    continueConversation,
    stopStreaming,
    deleteSession,
    approveToolCall,
    denyToolCall,
  } = sessionRuntime;
  const selectedPathNodes = useMemo(
    () => pathNodesToNode(activeConv, sessionRuntime.selectedPathHead),
    [activeConv, sessionRuntime.selectedPathHead]
  );
  const setPathHead = useCallback(
    async (nodeId) => {
      if (!activeConvId || !activeConv?.nodes?.[nodeId]) return null;
      const sessionHead = selectedSession?.currentHeadMessageId || selectedSession?.startHeadMessageId || null;
      setViewHeadId(nodeId === sessionHead ? null : nodeId);
      setSelectedNodeId(nodeId);
      await loadConversation(activeConvId, {
        headMessageId: nodeId,
        countTokens: false,
      });
      return nodeId;
    },
    [activeConv, activeConvId, loadConversation, selectedSession]
  );
  const activeContextSignatures = useMemo(
    () => contextSignatureParts(activeConv, activeModelId, selectedPathNodes),
    [activeConv, activeModelId, selectedPathNodes]
  );
  const activeCatalogModel = useMemo(() => models.find((m) => m.id === activeModelId) || null, [activeModelId, models]);
  const tokenMeter = useMemo(() => {
    const max = activeCatalogModel?.contextLength ?? activeCatalogModel?.maxInputTokens ?? null;
    const ic = inputTokenCounts[tokenCountKey(activeConv?.id, activeModelId)] || null;
    const cur = ic?.signature === activeContextSignatures.fullSignature ? ic : null;
    const post = latestAssistantTotalTokens(selectedPathNodes);
    const unavailable = cur?.source === "unsupported" || cur?.source === "unavailable";
    const used = unavailable ? null : cur?.inputTokens ?? post;
    return {
      used,
      max,
      model: activeModelId,
      measuredModel: cur?.model || null,
      source: cur?.inputTokens != null ? cur?.source || null : unavailable ? cur.source : used != null ? "postquery_total" : null,
    };
  }, [activeConv?.id, selectedPathNodes, activeContextSignatures.fullSignature, activeCatalogModel, activeModelId, inputTokenCounts]);

  useEffect(() => {
    if (!activeCatalogModel) return;
    loadModelParameters(activeModelId);
  }, [activeCatalogModel, activeModelId, loadModelParameters]);

  const activeModelParameters = useMemo(() => modelParametersById[activeModelId] || null, [activeModelId, modelParametersById]);

  const runMutation = useCallback(
    async (op, options = {}) => {
      try {
        const res = await op();
        setApiError(null);
        if (options.refreshList) await refreshConversations();
        if (options.reload !== false && activeConvId) await loadConversation(options.convId || activeConvId);
        return res;
      } catch (e) {
        if (isAbortError(e)) {
          setApiError(null);
          return null;
        }
        setApiError(e.message);
        toast.error(e.message);
        throw e;
      }
    },
    [activeConvId, loadConversation, refreshConversations]
  );

  const setConversationReasoningEffort = useCallback(
    (convId, effort) => {
      if (!convId) return Promise.resolve(null);
      return runMutation(
        async () => {
          const result = await setConversationReasoningApi(convId, effort || null);
          setConversations((prev) => prev.map((c) => (c.id === convId ? { ...c, reasoning: result.reasoning || null } : c)));
          return result;
        },
        { convId, reload: false, refreshList: false }
      );
    },
    [runMutation]
  );

  const selectConversation = useCallback((convId) => {
    setViewHeadId(null);
    setSelectedNodeId(null);
    setActiveConvId(convId);
  }, []);

  const createConversation = useCallback(async () => {
    const body = await runMutation(
      () => apiRequest("/api/conversations", {
        method: "POST",
        body: JSON.stringify({ model: activeConv?.model || null }),
      }),
      { reload: false, refreshList: true }
    );
    selectConversation(body.conversation_id);
    return body.conversation_id;
  }, [activeConv?.model, runMutation, selectConversation]);

  const renameConversation = useCallback(() => {
    toast.message("rename is not a Windie primitive yet");
  }, []);

  const deleteConversation = useCallback(
    async (convId) => {
      const wasActive = activeConvId === convId;
      await runMutation(() => apiRequest(`/api/conversations/${convId}`, { method: "DELETE" }), {
        reload: false,
        refreshList: false,
      });
      const summaries = await refreshConversations();
      if (wasActive) {
        selectConversation(summaries.find((c) => c.id !== convId)?.id || null);
      }
    },
    [activeConvId, refreshConversations, runMutation, selectConversation]
  );

  const setSystemPrompt = useCallback(
    (convId, text) => runMutation(() => apiRequest(`/api/conversations/${convId}/system-prompt`, { method: "PATCH", body: JSON.stringify({ text }) })),
    [runMutation]
  );

  const setConversationModel = useCallback(
    (convId, model) =>
      runMutation(
        async () => {
          const r = await setConversationModelApi(convId, model);
          loadModelParameters(model);
          return r;
        },
        { convId, refreshList: true }
      ),
    [loadModelParameters, runMutation]
  );

  const setToolApprovalMode = useCallback(
    (convId, mode) =>
      runMutation(() => apiRequest(`/api/conversations/${convId}/tool-approval-mode`, { method: "PATCH", body: JSON.stringify({ mode }) })),
    [runMutation]
  );

  const addToolSchema = useCallback((convId, s) => runMutation(() => apiRequest(`/api/conversations/${convId}/tools`, { method: "POST", body: JSON.stringify({ provider_id: s.providerId, tool_name: s.providerToolName }) })), [runMutation]);
  const addToolSchemas = useCallback(
    (convId, arr) =>
      runMutation(() => apiRequest(`/api/conversations/${convId}/tools/batch`, { method: "POST", body: JSON.stringify({ tools: arr.map((t) => ({ provider_id: t.providerId, tool_name: t.providerToolName })) }) })),
    [runMutation]
  );
  const removeToolSchema = useCallback((convId, name) => runMutation(() => apiRequest(`/api/conversations/${convId}/tools/${encodeURIComponent(name)}`, { method: "DELETE" })), [runMutation]);
  const removeToolSchemas = useCallback(
    (convId, names) =>
      runMutation(async () => {
        for (const n of names) await apiRequest(`/api/conversations/${convId}/tools/${encodeURIComponent(n)}`, { method: "DELETE" });
      }),
    [runMutation]
  );

  const runProviderAction = useCallback(
    async (action, providerId) => {
      try {
        const result = await action(providerId);
        setApiError(null);
        await refreshProviderInstallations();
        await refreshAvailableTools();
        return result;
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
        throw error;
      }
    },
    [refreshAvailableTools, refreshProviderInstallations]
  );

  const setupProvider = useCallback(
    (providerId) => runProviderAction(setupProviderApi, providerId),
    [runProviderAction]
  );
  const enableProvider = useCallback(
    (providerId) => runProviderAction(enableProviderApi, providerId),
    [runProviderAction]
  );
  const disableProvider = useCallback(
    (providerId) => runProviderAction(disableProviderApi, providerId),
    [runProviderAction]
  );
  const repairProvider = useCallback(
    (providerId) => runProviderAction(repairProviderApi, providerId),
    [runProviderAction]
  );
  const uninstallProvider = useCallback(
    (providerId) => runProviderAction(uninstallProviderApi, providerId),
    [runProviderAction]
  );

  const inspectNode = useCallback(
    (nodeId) => {
      // Tree selection is for inspecting a node. Session selection remains the
      // source of truth for the chat path and query target.
      setSelectedNodeId(nodeId);
    },
    []
  );

  const truncateAfter = useCallback(
    async (convId, nodeId) => {
      await runMutation(
        () => apiRequest(`/api/conversations/${convId}/truncate`, { method: "POST", body: JSON.stringify({ message_id: nodeId }) }),
        { reload: false }
      );
      const sessions = await refreshSessions();
      if (convId !== activeConvId) return sessions;

      const selected = getSelectedSession();
      const head =
        (selected?.conversationId === convId
          ? selected.currentHeadMessageId || selected.startHeadMessageId
          : null) || nodeId;
      await loadConversation(convId, {
        headMessageId: head,
        countTokens: false,
      });
      return sessions;
    },
    [activeConvId, getSelectedSession, loadConversation, refreshSessions, runMutation]
  );
  const removeMessage = useCallback(
    async (convId, nodeId) => {
      const conversation = conversations.find((item) => item.id === convId);
      const node = conversation?.nodes?.[nodeId];
      const parentHead = node?.parentId || null;
      const currentHead =
        viewHeadId ||
        selectedNodeRef.current ||
        selectedSession?.currentHeadMessageId ||
        selectedSession?.startHeadMessageId ||
        null;
      const nextHead = currentHead === nodeId ? parentHead : currentHead;

      await runMutation(
        () => apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, { method: "DELETE" }),
        { reload: false }
      );
      await refreshSessions();

      if (convId !== activeConvId) return;
      if (viewHeadId === nodeId) setViewHeadId(parentHead);
      setSelectedNodeId((current) => (current === nodeId ? parentHead : current));
      await loadConversation(convId, {
        headMessageId: nextHead,
        countTokens: false,
      });
    },
    [
      activeConvId,
      conversations,
      loadConversation,
      refreshSessions,
      runMutation,
      selectedSession,
      setViewHeadId,
      viewHeadId,
    ]
  );
  const editMessage = useCallback((convId, nodeId, text) => runMutation(() => apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, { method: "PATCH", body: JSON.stringify({ text }) })), [runMutation]);
  const forkFromMessage = useCallback(
    async (convId, nodeId) => {
      const body = await runMutation(() => apiRequest(`/api/conversations/${convId}/fork`, { method: "POST", body: JSON.stringify({ message_id: nodeId }) }), {
        reload: false,
        refreshList: true,
      });
      setSelectedNodeId(null);
      setActiveConvId(body.conversation_id);
      return body.conversation_id;
    },
    [runMutation]
  );

  const startGateway = useCallback(() => runMutation(async () => { const r = await apiRequest("/api/gateway/start", { method: "POST" }); await refreshGateway(); await refreshModels().catch(() => {}); return r; }, { reload: false }), [refreshGateway, refreshModels, runMutation]);
  const stopGateway = useCallback(() => runMutation(async () => { const r = await apiRequest("/api/gateway/stop", { method: "POST" }); await refreshGateway(); await refreshModels().catch(() => {}); return r; }, { reload: false }), [refreshGateway, refreshModels, runMutation]);
  const value = {
    conversations,
    activeConv,
    activeConvId,
    selectedNodeId,
    viewHeadId,
    selectedPathNodes,
    theme,
    treeOverlayOpen,
    contextPreviewOpen,
    streaming,
    pendingAssistant,
    sessionsById,
    selectedSession,
    selectedSessionId,
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
    availableToolsLoading,
    toolProviderStatuses,
    providerInstallations,
    providerInstallationsLoading,
    apiError,
    gatewayRunning,
    approvals,
    inspectNode,
    setPathHead,
    setSelectedNodeId,
    setTheme,
    setTreeOverlayOpen,
    setContextPreviewOpen,
    setSearchQuery,
    refreshModels,
    loadModelParameters,
    createConversation,
    selectConversation,
    selectSession,
    deleteSession,
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
    setupProvider,
    enableProvider,
    disableProvider,
    repairProvider,
    uninstallProvider,
    refreshProviderInstallations,
    truncateAfter,
    removeMessage,
    editMessage,
    forkFromMessage,
    sendMessage,
    continueConversation,
    stopStreaming,
    startGateway,
    stopGateway,
    refreshGateway,
    approveToolCall,
    denyToolCall,
    refreshSessions,
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
