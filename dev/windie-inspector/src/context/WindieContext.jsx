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
} from "@/lib/windieApi";
import { streamSessionEvents } from "@/lib/sessionStream";
import {
  conversationFromInspection,
  conversationSummaryFromApi,
  sessionFromApi,
  toolCatalogFromApi,
  toolProviderStatusesFromApi,
} from "@/lib/windieMappers";

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

function isLiveSession(session) {
  return session?.status === "running" || session?.status === "waiting_for_approval";
}

function isRunningSession(session) {
  return session?.status === "running";
}

function isAncestor(ancestorId, leafId, nodes) {
  if (!ancestorId || !leafId || !nodes) return false;
  if (ancestorId === leafId) return true;
  let cur = nodes[leafId];
  const seen = new Set();
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    if (cur.parentId === ancestorId) return true;
    cur = cur.parentId ? nodes[cur.parentId] : null;
  }
  return false;
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

function findLiveForLeaf(leafId, sessionsById, nodes, liveHeads) {
  if (!leafId || !nodes) return [];
  const out = [];
  for (const s of Object.values(sessionsById || {})) {
    if (!isRunningSession(s)) continue;
    const live = liveHeads?.[s.id] || s.currentHeadMessageId || s.startHeadMessageId;
    if (!live) continue;
    // session on path if its start is ancestor of leaf or live is descendant of leaf
    if (isAncestor(s.startHeadMessageId, leafId, nodes) || s.startHeadMessageId === leafId || isAncestor(leafId, live, nodes) || live === leafId) {
      out.push(s);
    }
  }
  out.sort((a, b) => (b.createdAt || 0) - (a.createdAt || 0));
  return out;
}

export function WindieProvider({ children }) {
  const [conversations, setConversations] = useState([]);
  const [activeConvId, setActiveConvId] = useState(null);
  const [selectedNodeId, setSelectedNodeId] = useState(null);
  const [theme, setTheme] = useState("dark");
  const [treeOverlayOpen, setTreeOverlayOpen] = useState(false);
  const [contextPreviewOpen, setContextPreviewOpen] = useState(false);
  const [inspectorPanelOpen, setInspectorPanelOpen] = useState(true);
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

  // path-centric model
  const subscriptionsRef = useRef(new Map()); // sessionId -> AbortController
  const followedSessionIdRef = useRef(null);
  const liveHeadBySessionRef = useRef({});
  const lastEventIdRef = useRef({});
  const activeConvRef = useRef(null);
  const sessionsRef = useRef({});
  const selectedNodeRef = useRef(null);

  useEffect(() => {
    activeConvRef.current = conversations.find((c) => c.id === activeConvId) || null;
  }, [conversations, activeConvId]);

  useEffect(() => {
    sessionsRef.current = sessionsById;
  }, [sessionsById]);

  useEffect(() => {
    selectedNodeRef.current = selectedNodeId;
  }, [selectedNodeId]);

  useEffect(
    () => () => {
      for (const c of subscriptionsRef.current.values()) c.abort();
      subscriptionsRef.current.clear();
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
    const body = await apiRequest("/api/tools");
    setAvailableToolSchemas(toolCatalogFromApi(body));
    setToolProviderStatuses(toolProviderStatusesFromApi(body));
    return toolCatalogFromApi(body);
  }, []);

  const loadConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId) return null;
      const hasHead = Object.prototype.hasOwnProperty.call(options, "headMessageId");
      const headMessageId = hasHead ? options.headMessageId : selectedNodeRef.current;
      const q = headMessageId ? `?head_message_id=${encodeURIComponent(headMessageId)}` : "";
      const [report, approvalBody] = await Promise.all([
        apiRequest(`/api/conversations/${convId}${q}`),
        apiRequest(`/api/conversations/${convId}/run-approvals`),
      ]);
      const loaded = conversationFromInspection(report, null);
      setConversations((prev) => {
        const fallback = prev.find((conv) => conv.id === convId);
        const withFallback = conversationFromInspection(report, fallback);
        return prev.some((c) => c.id === convId) ? prev.map((c) => (c.id === convId ? withFallback : c)) : [withFallback, ...prev];
      });
      if (options.selectLast !== false) {
        const last = loaded?.selectedPath?.[loaded.selectedPath.length - 1] || loaded?.rootId || null;
        setSelectedNodeId((cur) => (cur && loaded?.nodes?.[cur] ? cur : last));
      }
      setApprovals(approvalBody.approvals || []);

      if (options.countTokens !== false && loaded?.id) {
        const mid = loaded?.model || null;
        const sig = contextSignatureParts(loaded, mid).fullSignature;
        const key = tokenCountKey(loaded?.id, mid);
        const rid = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
        setInputTokenCounts((p) => ({ ...p, [key]: { ...(p[key] || {}), latestRequestId: rid } }));
        countConversationInputTokens(loaded.id, null, headMessageId || null)
          .then((count) => {
            setInputTokenCounts((p) => {
              if (p[key]?.latestRequestId !== rid) return p;
              return { ...p, [key]: { ...count, source: count.source || "prequery_input", signature: sig, latestRequestId: rid, measuredAt: Date.now() } };
            });
          })
          .catch(() => {
            setInputTokenCounts((p) => {
              if (p[key]?.latestRequestId !== rid) return p;
              return { ...p, [key]: { inputTokens: null, totalTokens: null, model: mid, raw: null, source: "prequery_input", signature: sig, latestRequestId: rid, measuredAt: Date.now() } };
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
    if (!activeConvId) return;
    let cancelled = false;
    loadConversation(activeConvId)
      .then(() => {
        if (!cancelled) setApiError(null);
      })
      .catch((e) => {
        if (!cancelled) setApiError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [activeConvId, loadConversation]);

  const activeConv = useMemo(() => conversations.find((c) => c.id === activeConvId) || null, [conversations, activeConvId]);
  const selectedPathNodes = useMemo(() => pathNodesToNode(activeConv, selectedNodeId), [activeConv, selectedNodeId]);
  const activeModelId = useMemo(() => activeConv?.model || null, [activeConv?.model]);
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
    const used = cur?.inputTokens ?? post;
    return {
      used,
      max,
      model: activeModelId,
      measuredModel: cur?.model || null,
      source: cur?.inputTokens != null ? cur?.source || null : used != null ? "postquery_total" : null,
    };
  }, [activeConv?.id, selectedPathNodes, activeContextSignatures.fullSignature, activeCatalogModel, activeModelId, inputTokenCounts]);

  useEffect(() => {
    if (!activeCatalogModel) return;
    loadModelParameters(activeModelId);
  }, [activeCatalogModel, activeModelId, loadModelParameters]);

  const activeModelParameters = useMemo(() => modelParametersById[activeModelId] || null, [activeModelId, modelParametersById]);
  const activeReasoning = activeConv?.reasoning || null;

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
    for (const c of subscriptionsRef.current.values()) c.abort();
    subscriptionsRef.current.clear();
    followedSessionIdRef.current = null;
    setVisibleSessionId(null);
    setSelectedNodeId(null);
    setActiveConvId(convId);
  }, []);

  const createConversation = useCallback(async () => {
    const body = await runMutation(() => apiRequest("/api/conversations", { method: "POST" }), { reload: false, refreshList: true });
    selectConversation(body.conversation_id);
    await loadConversation(body.conversation_id, { headMessageId: null });
    return body.conversation_id;
  }, [loadConversation, runMutation, selectConversation]);

  const renameConversation = useCallback(() => {
    toast.message("rename is not a Windie primitive yet");
  }, []);

  const deleteConversation = useCallback(
    async (convId) => {
      await runMutation(() => apiRequest(`/api/conversations/${convId}`, { method: "DELETE" }), {
        reload: false,
        refreshList: false,
      });
      const summaries = await refreshConversations();
      selectConversation(summaries.find((c) => c.id !== convId)?.id || null);
    },
    [refreshConversations, runMutation, selectConversation]
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

  const rememberSession = useCallback((session) => {
    const norm = sessionFromApi(session);
    if (!norm) return null;
    setSessionsById((p) => ({ ...p, [norm.id]: norm }));
    sessionsRef.current = { ...sessionsRef.current, [norm.id]: norm };
    return norm;
  }, []);

  const abortAll = useCallback(() => {
    for (const c of subscriptionsRef.current.values()) c.abort();
    subscriptionsRef.current.clear();
    followedSessionIdRef.current = null;
    setVisibleSessionId(null);
  }, []);

  const inspectNode = useCallback(
    (nodeId) => {
      // left tree = inspect only, unsubscribe
      if (followedSessionIdRef.current) {
        for (const c of subscriptionsRef.current.values()) c.abort();
        subscriptionsRef.current.clear();
        followedSessionIdRef.current = null;
        setVisibleSessionId(null);
        toast.message("unfollowed · inspecting message");
      } else {
        abortAll();
      }
      setSelectedNodeId(nodeId);
    },
    [abortAll]
  );

  const handleEvent = useCallback(
    async (session, data) => {
      if (!data?.type) return;
      setSessionEventsBySessionId((p) => ({ ...p, [session.id]: [...(p[session.id] || []), data] }));

      if (data.type === "assistant_delta") {
        setPendingAssistantBySessionId((p) => {
          const cur = p[session.id] || { convId: session.conversationId, text: "", reasoning: "", toolCalls: {} };
          return { ...p, [session.id]: { ...cur, text: cur.text + (data.text || "") } };
        });
        return;
      }
      if (data.type === "reasoning_delta") {
        setPendingAssistantBySessionId((p) => {
          const cur = p[session.id] || { convId: session.conversationId, text: "", reasoning: "", toolCalls: {} };
          return { ...p, [session.id]: { ...cur, reasoning: (cur.reasoning || "") + (data.text || "") } };
        });
        return;
      }
      if (data.type === "tool_call_delta") {
        setPendingAssistantBySessionId((p) => {
          const cur = p[session.id] || { convId: session.conversationId, text: "", reasoning: "", toolCalls: {} };
          const idx = String(data.index ?? 0);
          const ex = cur.toolCalls?.[idx] || { id: null, name: null, argumentsText: "" };
          return {
            ...p,
            [session.id]: {
              ...cur,
              toolCalls: { ...(cur.toolCalls || {}), [idx]: { id: data.id || ex.id, name: data.name || ex.name, argumentsText: ex.argumentsText + (data.arguments_delta || "") } },
            },
          };
        });
        return;
      }

      if (data.type === "assistant_message_saved" || data.type === "tool_result_saved") {
        if (data.message_id) liveHeadBySessionRef.current[session.id] = data.message_id;
        const followed = followedSessionIdRef.current === session.id;
        if (followed) {
          // follow this path: move viewport to new leaf
          await loadConversation(session.conversationId, { countTokens: false, selectLast: false, headMessageId: data.message_id || null }).catch(() => {});
          if (data.message_id) setSelectedNodeId(data.message_id);
          setPendingAssistantBySessionId((p) => ({ ...p, [session.id]: null }));
        } else {
          // background: silent refresh without pulling viewport
          await loadConversation(session.conversationId, { countTokens: false, selectLast: false, headMessageId: undefined }).catch(() => {});
        }
        return;
      }

      if (data.type === "completed" || data.type === "failed" || data.type === "cancelled" || data.type === "waiting_for_approval") {
        const latest = await getSession(session.id).catch(() => null);
        if (latest) rememberSession(latest);
        if (data.type === "completed" && data.message_id) liveHeadBySessionRef.current[session.id] = data.message_id;
        const followed = followedSessionIdRef.current === session.id;
        if (followed) {
          if (data.type === "completed" && data.message_id) {
            await loadConversation(session.conversationId, { countTokens: false, selectLast: false, headMessageId: data.message_id }).catch(() => {});
            setSelectedNodeId(data.message_id);
          } else {
            await loadConversation(session.conversationId, { countTokens: false, selectLast: false, headMessageId: undefined }).catch(() => {});
          }
        } else {
          await loadConversation(session.conversationId, { countTokens: false, selectLast: false, headMessageId: undefined }).catch(() => {});
        }
        if (data.type !== "waiting_for_approval") setPendingAssistantBySessionId((p) => ({ ...p, [session.id]: null }));
      }
    },
    [loadConversation, rememberSession]
  );

  const subscribeToSession = useCallback(
    (session) => {
      const norm = rememberSession(session);
      if (!norm) return null;
      for (const c of subscriptionsRef.current.values()) c.abort();
      subscriptionsRef.current.clear();
      followedSessionIdRef.current = norm.id;
      setVisibleSessionId(norm.id);
      const controller = new AbortController();
      subscriptionsRef.current.set(norm.id, controller);
      streamSessionEvents(
        norm.id,
        lastEventIdRef.current[norm.id] ?? null,
        async ({ id, data }) => {
          await handleEvent(norm, data);
          const eid = id ?? data?.event_id ?? null;
          if (eid != null) lastEventIdRef.current[norm.id] = eid;
        },
        { signal: controller.signal }
      )
        .catch((e) => {
          if (!isAbortError(e)) {
            setApiError(e.message);
            toast.error(e.message);
          }
        })
        .finally(() => {
          if (subscriptionsRef.current.get(norm.id) === controller) subscriptionsRef.current.delete(norm.id);
        });
      return norm;
    },
    [handleEvent, rememberSession]
  );

  const subscribeToPathLeaf = useCallback(
    async (leafId, convIdOverride) => {
      if (!leafId) return null;
      const convId = convIdOverride || activeConvRef.current?.id;
      if (!convId) return null;
      const conv = activeConvRef.current;
      const cand = findLiveForLeaf(leafId, sessionsRef.current, conv?.nodes, liveHeadBySessionRef.current);
      const sess = cand[0] || null;
      // unsubscribe old
      for (const c of subscriptionsRef.current.values()) c.abort();
      subscriptionsRef.current.clear();
      followedSessionIdRef.current = sess ? sess.id : null;
      setVisibleSessionId(sess ? sess.id : null);
      setSelectedNodeId(leafId);
      await loadConversation(convId, { headMessageId: leafId, selectLast: false, countTokens: false }).catch(() => {});
      if (sess) {
        subscribeToSession(sess);
        return sess;
      }
      return null;
    },
    [loadConversation, subscribeToSession]
  );

  const selectPathHead = useCallback(async (_convId, leafId) => subscribeToPathLeaf(leafId, _convId), [subscribeToPathLeaf]);
  const selectPath = useCallback((convId, path) => selectPathHead(convId, path[path.length - 1]), [selectPathHead]);

  const truncateAfter = useCallback((convId, nodeId) => runMutation(() => apiRequest(`/api/conversations/${convId}/truncate`, { method: "POST", body: JSON.stringify({ message_id: nodeId }) })), [runMutation]);
  const removeMessage = useCallback((convId, nodeId) => runMutation(() => apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, { method: "DELETE" })), [runMutation]);
  const editMessage = useCallback((convId, nodeId, text) => runMutation(() => apiRequest(`/api/conversations/${convId}/messages/${nodeId}`, { method: "PATCH", body: JSON.stringify({ text }) })), [runMutation]);
  const forkFromMessage = useCallback(
    async (convId, nodeId) => {
      const body = await runMutation(() => apiRequest(`/api/conversations/${convId}/fork`, { method: "POST", body: JSON.stringify({ message_id: nodeId }) }), {
        reload: false,
        refreshList: true,
      });
      abortAll();
      setSelectedNodeId(null);
      setActiveConvId(body.conversation_id);
      await loadConversation(body.conversation_id);
      return body.conversation_id;
    },
    [loadConversation, runMutation, abortAll]
  );

  const refreshSessions = useCallback(async () => {
    const sessions = (await listSessions()).map(sessionFromApi).filter(Boolean);
    setSessionsById(Object.fromEntries(sessions.map((s) => [s.id, s])));
    sessionsRef.current = Object.fromEntries(sessions.map((s) => [s.id, s]));
    return sessions;
  }, []);

  useEffect(() => {
    refreshSessions().catch((e) => setApiError(e.message));
  }, [refreshSessions]);

  const stopStreaming = useCallback(async () => {
    if (!visibleSessionId) return;
    try {
      const s = await stopSessionApi(visibleSessionId);
      rememberSession(s);
      setPendingAssistantBySessionId((p) => ({ ...p, [visibleSessionId]: null }));
      abortAll();
    } catch (e) {
      setApiError(e.message);
      toast.error(e.message);
    }
  }, [rememberSession, visibleSessionId, abortAll]);

  const sendMessage = useCallback(
    async (convId, text, options = {}) => {
      const att = options.attachments || [];
      if (!text.trim() && att.length === 0) return;
      const conv = activeConvRef.current;
      const leaf = conv?.selectedPath?.[conv?.selectedPath.length - 1] || selectedNodeRef.current || null;
      if (leaf) {
        const busy = findLiveForLeaf(leaf, sessionsRef.current, conv?.nodes, liveHeadBySessionRef.current);
        if (busy.length > 0) {
          toast.message("path busy", { description: "wait for running agent on this path" });
          return;
        }
      }
      try {
        const parts = await messagePartsForSend(text, att);
        const parentId = selectedNodeRef.current || null;
        const ins = await apiRequest(`/api/conversations/${convId}/messages`, {
          method: "POST",
          body: JSON.stringify({ head_message_id: parentId, role: "user", parts }),
        });
        // Fix flash: stay on parent path until new branch exists locally.
        // Previously we setSelectedNodeId(ins.message_id) BEFORE load, which made
        // pathNodesToNode fall back to conv.selectedPath (old unrelated path) for a frame.
        await loadConversation(convId, { headMessageId: ins.message_id, selectLast: false, countTokens: false });
        setSelectedNodeId(ins.message_id);
        const session = await createSessionApi(convId, { headMessageId: ins.message_id, model: activeConv?.model || null, reasoning: activeReasoning });
        subscribeToSession(session);
        setApiError(null);
      } catch (e) {
        setApiError(e.message);
        toast.error(e.message);
      }
    },
    [activeConv?.model, activeReasoning, loadConversation, subscribeToSession]
  );

  const continueConversation = useCallback(
    async (convId) => {
      if (!convId) return;
      const conv = activeConvRef.current;
      const leaf = conv?.selectedPath?.[conv?.selectedPath.length - 1] || selectedNodeRef.current || null;
      if (leaf) {
        const busy = findLiveForLeaf(leaf, sessionsRef.current, conv?.nodes, liveHeadBySessionRef.current);
        if (busy.length > 0) {
          toast.message("path busy", { description: "wait for running agent on this path" });
          return;
        }
      }
      try {
        const session = await createSessionApi(convId, { headMessageId: selectedNodeRef.current, model: activeConv?.model || null, reasoning: activeReasoning });
        subscribeToSession(session);
        setApiError(null);
      } catch (e) {
        setApiError(e.message);
        toast.error(e.message);
      }
    },
    [activeConv?.model, activeReasoning, subscribeToSession]
  );

  const startGateway = useCallback(() => runMutation(async () => { const r = await apiRequest("/api/gateway/start", { method: "POST" }); await refreshGateway(); await refreshModels().catch(() => {}); return r; }, { reload: false }), [refreshGateway, refreshModels, runMutation]);
  const stopGateway = useCallback(() => runMutation(async () => { const r = await apiRequest("/api/gateway/stop", { method: "POST" }); await refreshGateway(); await refreshModels().catch(() => {}); return r; }, { reload: false }), [refreshGateway, refreshModels, runMutation]);
  const approveToolCall = useCallback(
    async (sid, tcid) => {
      if (!sid) return;
      try {
        const s = await approveSessionToolApi(sid, tcid);
        subscribeToSession(s);
      } catch (e) {
        setApiError(e.message);
        toast.error(e.message);
      }
    },
    [subscribeToSession]
  );
  const denyToolCall = useCallback(
    async (sid, tcid) => {
      if (!sid) return;
      try {
        const s = await denySessionToolApi(sid, tcid);
        subscribeToSession(s);
      } catch (e) {
        setApiError(e.message);
        toast.error(e.message);
      }
    },
    [subscribeToSession]
  );

  const visibleSession = visibleSessionId ? sessionsById[visibleSessionId] || null : null;
  const streaming = isLiveSession(visibleSession);
  const pendingAssistant = visibleSessionId ? pendingAssistantBySessionId[visibleSessionId] || null : null;

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
    isAncestor,
    subscribeToPathLeaf,
    inspectNode,
    abortAllSubscriptions: abortAll,
    setSelectedNodeId,
    setTheme,
    setTreeOverlayOpen,
    setContextPreviewOpen,
    setSearchQuery,
    refreshModels,
    loadModelParameters,
    inspectorPanelOpen,
    setInspectorPanelOpen,
    createConversation,
    selectConversation,
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
