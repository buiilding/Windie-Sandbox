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
  conversationFromInspection,
  conversationSummaryFromApi,
  approveSessionTool as approveSessionToolApi,
  createSession as createSessionApi,
  denySessionTool as denySessionToolApi,
  fetchModelParameters,
  getSession,
  listSessions,
  listModels,
  setConversationModel as setConversationModelApi,
  setConversationReasoning as setConversationReasoningApi,
  stopSession as stopSessionApi,
  streamSessionEvents,
  toolCatalogFromApi,
  toolProviderStatusesFromApi,
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

function isAbortError(error) {
  return error?.name === "AbortError";
}

function sessionFromApi(session) {
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

function isLiveSession(session) {
  return session?.status === "running" || session?.status === "waiting_for_approval";
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
    return {
      pathSignature: "",
      setupSignature: "",
      fullSignature: "",
    };
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
  const [searchQuery, setSearchQuery] = useState("");
  const [apiError, setApiError] = useState(null);
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [approvals, setApprovals] = useState([]);
  const [availableToolSchemas, setAvailableToolSchemas] = useState([]);
  const [toolProviderStatuses, setToolProviderStatuses] = useState([]);
  const [models, setModels] = useState([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(null);
  const [inputTokenCounts, setInputTokenCounts] = useState({});
  const [modelParametersById, setModelParametersById] = useState({});
  const [sessionsById, setSessionsById] = useState({});
  const [visibleSessionId, setVisibleSessionId] = useState(null);
  const [sessionEventsBySessionId, setSessionEventsBySessionId] = useState({});
  const [pendingAssistantBySessionId, setPendingAssistantBySessionId] = useState({});
  const subscriptionAbortRef = useRef(null);
  const subscribedSessionIdRef = useRef(null);
  const visibleSessionIdRef = useRef(null);

  useEffect(
    () => () => {
      subscriptionAbortRef.current?.abort();
      subscribedSessionIdRef.current = null;
      visibleSessionIdRef.current = null;
    },
    []
  );

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
      setModelParametersById((prev) => ({
        ...prev,
        [modelId]: { status: "ready", data, error: null },
      }));
      return data;
    } catch (error) {
      setModelParametersById((prev) => ({
        ...prev,
        [modelId]: { status: "error", data: null, error: error.message },
      }));
      return null;
    }
  }, [gatewayRunning, modelParametersById, models, modelsError, modelsLoading]);

  const refreshAvailableTools = useCallback(async () => {
    const body = await apiRequest("/api/tools");
    const tools = toolCatalogFromApi(body);
    const providers = toolProviderStatusesFromApi(body);
    setAvailableToolSchemas(tools);
    setToolProviderStatuses(providers);
    return tools;
  }, []);

  const loadConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId) return null;
      const headMessageId = options.headMessageId ?? selectedNodeId;
      const inspectQuery = headMessageId
        ? `?head_message_id=${encodeURIComponent(headMessageId)}`
        : "";
      const [report, approvalBody] = await Promise.all([
        apiRequest(`/api/conversations/${convId}${inspectQuery}`),
        apiRequest(`/api/conversations/${convId}/session-approvals`),
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
        const last = loaded?.selectedPath?.[loaded.selectedPath.length - 1] || loaded?.rootId || null;
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

        countConversationInputTokens(loaded.id, null, headMessageId || null)
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
    [selectedNodeId]
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

  const selectedPathNodes = useMemo(() => {
    return pathNodesToNode(activeConv, selectedNodeId);
  }, [activeConv, selectedNodeId]);

  const activeModelId = useMemo(
    () => activeConv?.model || null,
    [activeConv?.model]
  );

  const activeContextSignatures = useMemo(
    () => contextSignatureParts(activeConv, activeModelId, selectedPathNodes),
    [activeConv, activeModelId, selectedPathNodes]
  );

  const activeCatalogModel = useMemo(
    () => models.find((model) => model.id === activeModelId) || null,
    [activeModelId, models]
  );

  const tokenMeter = useMemo(() => {
    const maxTokens =
      activeCatalogModel?.contextLength ?? activeCatalogModel?.maxInputTokens ?? null;
    const inputCount = inputTokenCounts[tokenCountKey(activeConv?.id, activeModelId)] || null;
    const currentInputCount =
      inputCount?.signature === activeContextSignatures.fullSignature ? inputCount : null;
    const postQueryTotalTokens = latestAssistantTotalTokens(selectedPathNodes);
    const used = currentInputCount?.inputTokens ?? postQueryTotalTokens;

    return {
      used,
      max: maxTokens,
      model: activeModelId,
      measuredModel: currentInputCount?.model || null,
      source:
        currentInputCount?.inputTokens != null
          ? currentInputCount?.source || null
          : used != null
            ? "postquery_total"
            : null,
    };
  }, [
    activeConv?.id,
    selectedPathNodes,
    activeContextSignatures.fullSignature,
    activeCatalogModel,
    activeModelId,
    inputTokenCounts,
  ]);

  useEffect(() => {
    if (!activeCatalogModel) return;
    loadModelParameters(activeModelId);
  }, [activeCatalogModel, activeModelId, loadModelParameters]);

  const activeModelParameters = useMemo(
    () => modelParametersById[activeModelId] || null,
    [activeModelId, modelParametersById]
  );

  const activeReasoning = activeConv?.reasoning || null;

  const runMutation = useCallback(
    async (operation, options = {}) => {
      try {
        const result = await operation();
        setApiError(null);
        if (options.refreshList) await refreshConversations();
        if (options.reload !== false && activeConvId) await loadConversation(options.convId || activeConvId);
        return result;
      } catch (error) {
        if (isAbortError(error)) {
          setApiError(null);
          return null;
        }
        setApiError(error.message);
        toast.error(error.message);
        throw error;
      }
    },
    [activeConvId, loadConversation, refreshConversations]
  );

  const setConversationReasoningEffort = useCallback(
    (convId, effort) => {
      if (!convId) return Promise.resolve(null);

      return runMutation(async () => {
        const result = await setConversationReasoningApi(convId, effort || null);
        setConversations((prev) =>
          prev.map((conv) =>
            conv.id === convId ? { ...conv, reasoning: result.reasoning || null } : conv
          )
        );
        return result;
      }, {
        convId,
        reload: false,
        refreshList: false,
      });
    },
    [runMutation]
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
          body: JSON.stringify({
            text,
            head_message_id: selectedNodeId || null,
          }),
        })
      ),
    [runMutation, selectedNodeId]
  );

  const setConversationModel = useCallback(
    (convId, model) =>
      runMutation(async () => {
        const result = await setConversationModelApi(convId, model);
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
            head_message_id: selectedNodeId || null,
          }),
        })
      ),
    [runMutation, selectedNodeId]
  );

  const addToolSchemas = useCallback(
    (convId, toolSchemas) =>
      runMutation(() =>
        apiRequest(`/api/conversations/${convId}/tools/batch`, {
          method: "POST",
          body: JSON.stringify({
            head_message_id: selectedNodeId || null,
            tools: toolSchemas.map((toolSchema) => ({
              provider_id: toolSchema.providerId,
              tool_name: toolSchema.providerToolName,
            })),
          }),
        })
      ),
    [runMutation, selectedNodeId]
  );

  const removeToolSchema = useCallback(
    (convId, name) =>
      runMutation(() =>
        apiRequest(
          `/api/conversations/${convId}/tools/${encodeURIComponent(name)}${
            selectedNodeId ? `?head_message_id=${encodeURIComponent(selectedNodeId)}` : ""
          }`,
          {
            method: "DELETE",
          }
        )
      ),
    [runMutation, selectedNodeId]
  );

  const removeToolSchemas = useCallback(
    (convId, names) =>
      runMutation(async () => {
        for (const name of names) {
          await apiRequest(
            `/api/conversations/${convId}/tools/${encodeURIComponent(name)}${
              selectedNodeId ? `?head_message_id=${encodeURIComponent(selectedNodeId)}` : ""
            }`,
            {
              method: "DELETE",
            }
          );
        }
      }),
    [runMutation, selectedNodeId]
  );

  const selectPathHead = useCallback(
    (_convId, leafId) => {
      setSelectedNodeId(leafId);
      return Promise.resolve();
    },
    []
  );

  const selectPath = useCallback(
    (convId, path) => {
      const leafId = path[path.length - 1];
      if (!leafId) return Promise.resolve();
      return selectPathHead(convId, leafId);
    },
    [selectPathHead]
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

  const rememberSession = useCallback((session) => {
    const normalized = sessionFromApi(session);
    if (!normalized) return null;
    setSessionsById((prev) => ({ ...prev, [normalized.id]: normalized }));
    return normalized;
  }, []);

  const handleSessionEvent = useCallback(
    async (session, data) => {
      if (!data?.type) return;

      setSessionEventsBySessionId((prev) => ({
        ...prev,
        [session.id]: [...(prev[session.id] || []), data],
      }));

      if (data.type === "assistant_delta") {
        setPendingAssistantBySessionId((prev) => {
          const current = prev[session.id] || {
            convId: session.conversationId,
            text: "",
            reasoning: "",
            toolCalls: {},
          };
          return {
            ...prev,
            [session.id]: { ...current, text: current.text + (data.text || "") },
          };
        });
        return;
      }

      if (data.type === "reasoning_delta") {
        setPendingAssistantBySessionId((prev) => {
          const current = prev[session.id] || {
            convId: session.conversationId,
            text: "",
            reasoning: "",
            toolCalls: {},
          };
          return {
            ...prev,
            [session.id]: {
              ...current,
              reasoning: (current.reasoning || "") + (data.text || ""),
            },
          };
        });
        return;
      }

      if (data.type === "tool_call_delta") {
        setPendingAssistantBySessionId((prev) => {
          const current = prev[session.id] || {
            convId: session.conversationId,
            text: "",
            reasoning: "",
            toolCalls: {},
          };
          const index = String(data.index ?? 0);
          const existing = current.toolCalls?.[index] || {
            id: null,
            name: null,
            argumentsText: "",
          };
          return {
            ...prev,
            [session.id]: {
              ...current,
              toolCalls: {
                ...(current.toolCalls || {}),
                [index]: {
                  id: data.id || existing.id,
                  name: data.name || existing.name,
                  argumentsText:
                    existing.argumentsText + (data.arguments_delta || ""),
                },
              },
            },
          };
        });
        return;
      }

      if (data.type === "assistant_message_saved" || data.type === "tool_result_saved") {
        await loadConversation(session.conversationId, {
          countTokens: false,
          selectLast: false,
          headMessageId: data.message_id || null,
        });
        if (visibleSessionIdRef.current === session.id && data.message_id) {
          setSelectedNodeId(data.message_id);
        }
        setPendingAssistantBySessionId((prev) => ({ ...prev, [session.id]: null }));
        return;
      }

      if (
        data.type === "completed" ||
        data.type === "failed" ||
        data.type === "cancelled" ||
        data.type === "waiting_for_approval"
      ) {
        const latest = await getSession(session.id).catch(() => null);
        if (latest) rememberSession(latest);
        await loadConversation(session.conversationId, { countTokens: false }).catch(() => {});
        if (data.type !== "waiting_for_approval") {
          setPendingAssistantBySessionId((prev) => ({ ...prev, [session.id]: null }));
        }
      }
    },
    [loadConversation, rememberSession]
  );

  const subscribeToSession = useCallback(
    (session) => {
      const normalized = rememberSession(session);
      if (!normalized) return;

      visibleSessionIdRef.current = normalized.id;
      setVisibleSessionId(normalized.id);
      if (subscribedSessionIdRef.current === normalized.id) return;

      subscriptionAbortRef.current?.abort();
      const controller = new AbortController();
      subscriptionAbortRef.current = controller;
      subscribedSessionIdRef.current = normalized.id;

      streamSessionEvents(
        normalized.id,
        null,
        ({ data }) => handleSessionEvent(normalized, data),
        { signal: controller.signal }
      ).catch((error) => {
        if (!isAbortError(error)) {
          setApiError(error.message);
          toast.error(error.message);
        }
      }).finally(() => {
        if (subscriptionAbortRef.current === controller) {
          subscriptionAbortRef.current = null;
          subscribedSessionIdRef.current = null;
        }
      });
    },
    [handleSessionEvent, rememberSession]
  );

  const refreshSessions = useCallback(async () => {
    const sessions = (await listSessions()).map(sessionFromApi).filter(Boolean);
    setSessionsById(Object.fromEntries(sessions.map((session) => [session.id, session])));
    const visibleLiveSession =
      sessions.find((session) => session.conversationId === activeConvId && isLiveSession(session)) ||
      sessions.find(isLiveSession) ||
      null;
    if (visibleLiveSession) {
      subscribeToSession(visibleLiveSession);
    }
    return sessions;
  }, [activeConvId, subscribeToSession]);

  useEffect(() => {
    refreshSessions().catch((error) => setApiError(error.message));
  }, [refreshSessions]);

  const stopStreaming = useCallback(async () => {
    if (!visibleSessionId) return;
    try {
      const session = await stopSessionApi(visibleSessionId);
      rememberSession(session);
      setPendingAssistantBySessionId((prev) => ({ ...prev, [visibleSessionId]: null }));
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [rememberSession, visibleSessionId]);

  const sendMessage = useCallback(
    async (convId, text, options = {}) => {
      const attachments = options.attachments || [];
      if (!text.trim() && attachments.length === 0) return;
      try {
        const parts = await messagePartsForSend(text, attachments);
        const inserted = await apiRequest(`/api/conversations/${convId}/messages`, {
          method: "POST",
          body: JSON.stringify({
            head_message_id: selectedNodeId || null,
            role: "user",
            parts,
          }),
        });
        setSelectedNodeId(inserted.message_id);
        await loadConversation(convId, { headMessageId: inserted.message_id });
        const session = await createSessionApi(convId, {
          headMessageId: inserted.message_id,
          model: activeConv?.model || null,
          reasoning: activeReasoning,
        });
        subscribeToSession(session);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [activeConv?.model, activeReasoning, loadConversation, selectedNodeId, subscribeToSession]
  );

  const continueConversation = useCallback(
    async (convId) => {
      if (!convId) return;
      try {
        const session = await createSessionApi(convId, {
          headMessageId: selectedNodeId,
          model: activeConv?.model || null,
          reasoning: activeReasoning,
        });
        subscribeToSession(session);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [activeConv?.model, activeReasoning, selectedNodeId, subscribeToSession]
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
    async (sessionId, toolCallId) => {
      if (!sessionId) return;
      try {
        const session = await approveSessionToolApi(sessionId, toolCallId);
        subscribeToSession(session);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [subscribeToSession]
  );

  const denyToolCall = useCallback(
    async (sessionId, toolCallId) => {
      if (!sessionId) return;
      try {
        const session = await denySessionToolApi(sessionId, toolCallId);
        subscribeToSession(session);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [subscribeToSession]
  );

  const visibleSession = visibleSessionId ? sessionsById[visibleSessionId] || null : null;
  const streaming = isLiveSession(visibleSession);
  const pendingAssistant = visibleSessionId
    ? pendingAssistantBySessionId[visibleSessionId] || null
    : null;

  const value = {
    conversations,
    activeConv,
    activeConvId,
    selectedNodeId,
    selectedPathNodes,
    theme,
    treeOverlayOpen,
    contextPreviewOpen,
    streaming,
    pendingAssistant,
    sessionsById,
    visibleSession,
    visibleSessionId,
    sessionEventsBySessionId,
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
    toolProviderStatuses,
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
    selectPath,
    selectPathHead,
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
