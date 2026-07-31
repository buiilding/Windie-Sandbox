import {
  createContext,
  useContext,
  useMemo,
  useState,
  useCallback,
} from "react";
import { toast } from "sonner";
import {
  apiRequest,
  setConversationModel as setConversationModelApi,
  setConversationReasoning as setConversationReasoningApi,
} from "@/lib/windieApi";
import { useConversationStore } from "@/hooks/useConversationStore";
import { useGatewayStore } from "@/hooks/useGatewayStore";
import { useInspectorState } from "@/hooks/useInspectorState";
import { useModelCatalog } from "@/hooks/useModelCatalog";
import { useSessionRuntime } from "@/hooks/useSessionRuntime";
import { useToolCatalog } from "@/hooks/useToolCatalog";
import {
  contextSignatureParts,
  pathNodesForConversation,
} from "@/lib/conversationContext";

const WindieCtx = createContext(null);

function tokenCountKey(conversationId, modelId) {
  return `${conversationId || ""}::${modelId || ""}`;
}

function isAbortError(error) {
  return error?.name === "AbortError";
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

export function WindieProvider({ children }) {
  const {
    theme,
    contextPreviewOpen,
    searchQuery,
    apiError,
    setTheme,
    setContextPreviewOpen,
    setSearchQuery,
    setApiError,
  } = useInspectorState();
  const [approvals, setApprovals] = useState([]);

  const {
    conversations,
    activeConv,
    activeConvId,
    selectedNodeId,
    viewHeadId,
    inputTokenCounts,
    setActiveConvId,
    setSelectedNodeId,
    setViewHeadId,
    applySessionMessage,
    updateConversation,
    refreshConversations,
    loadConversation,
    selectConversation,
  } = useConversationStore({ setApiError, setApprovals });

  const activeModelId = activeConv?.model || null;
  const activeReasoning = activeConv?.reasoning || null;
  const handleResourceError = useCallback(
    (error) => setApiError(error.message),
    [setApiError]
  );
  const { gatewayRunning } = useGatewayStore({
    onError: handleResourceError,
  });
  const {
    models,
    modelsLoading,
    modelsError,
    modelParametersById,
    activeCatalogModel,
    activeModelParameters,
    refreshModels,
    loadModelParameters,
  } = useModelCatalog({ gatewayRunning, activeModelId });
  const {
    availableToolSchemas,
    availableToolsLoading,
    toolProviderStatuses,
    providerInstallations,
    providerInstallationsLoading,
    refreshAvailableTools,
    refreshProviderInstallations,
    setupProvider,
    enableProvider,
    disableProvider,
    repairProvider,
    uninstallProvider,
  } = useToolCatalog({
    onError: handleResourceError,
  });
  const sessionRuntime = useSessionRuntime({
    conversationId: activeConvId,
    viewHeadId,
    setViewHeadId,
    selectedNodeId,
    setSelectedNodeId,
    loadConversation,
    applySessionMessage,
    setApiError,
  });
  const {
    sessionsById,
    selectedSession,
    selectedSessionId,
    sessionResolution,
    getSelectedSession,
    pendingAssistant,
    streaming,
    refreshSessions,
    selectSession,
    resolvePathHead,
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
  const protectedMessageIds = useMemo(() => {
    const protectedIds = new Set();
    Object.values(sessionsById).forEach((session) => {
      if (session?.conversationId !== activeConv?.id) return;
      for (const messageId of session.protectedMessageIds || []) {
        protectedIds.add(messageId);
      }
    });
    return protectedIds;
  }, [activeConv?.id, sessionsById]);
  const setPathHead = useCallback(
    async (nodeId) => {
      if (!activeConvId || !activeConv?.nodes?.[nodeId]) return null;
      setSelectedNodeId(nodeId);
      try {
        const resolution = await resolvePathHead(nodeId);
        if (resolution.kind === "existing") return nodeId;

        setViewHeadId(nodeId);
        await loadConversation(activeConvId, {
          headMessageId: nodeId,
          countTokens: false,
        });
        return nodeId;
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
        return null;
      }
    },
    [activeConv, activeConvId, loadConversation, resolvePathHead, setApiError, setSelectedNodeId, setViewHeadId]
  );
  const activeContextSignatures = useMemo(
    () => contextSignatureParts(activeConv, activeModelId, selectedPathNodes),
    [activeConv, activeModelId, selectedPathNodes]
  );
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
    [activeConvId, loadConversation, refreshConversations, setApiError]
  );

  const setConversationReasoningEffort = useCallback(
    (convId, effort) => {
      if (!convId) return Promise.resolve(null);
      return runMutation(
        async () => {
          const result = await setConversationReasoningApi(convId, effort || null);
          updateConversation(convId, (conversation) => ({
            ...conversation,
            reasoning: result.reasoning || null,
          }));
          return result;
        },
        { convId, reload: false, refreshList: false }
      );
    },
    [runMutation, updateConversation]
  );

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

  const inspectNode = useCallback(
    (nodeId) => {
      // Tree selection is for inspecting a node. Session selection remains the
      // source of truth for the chat path and query target.
      setSelectedNodeId(nodeId);
    },
    [setSelectedNodeId]
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
          ? selected.currentHeadMessageId
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
        selectedNodeId ||
        selectedSession?.currentHeadMessageId ||
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
      selectedNodeId,
      selectedSession,
      setSelectedNodeId,
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
    [runMutation, setActiveConvId, setSelectedNodeId]
  );

  const value = {
    conversations,
    activeConv,
    activeConvId,
    selectedNodeId,
    viewHeadId,
    selectedPathNodes,
    protectedMessageIds,
    theme,
    contextPreviewOpen,
    streaming,
    pendingAssistant,
    sessionsById,
    selectedSession,
    selectedSessionId,
    sessionResolution,
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
