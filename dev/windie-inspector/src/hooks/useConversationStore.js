import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  apiRequest,
  countConversationInputTokens,
} from "@/lib/windieApi";
import {
  conversationFromInspection,
  conversationSummaryFromApi,
  upsertConversationMessage,
} from "@/lib/windieMappers";
import { contextSignatureParts } from "@/lib/conversationContext";

function tokenCountKey(conversationId, modelId) {
  return `${conversationId || ""}::${modelId || ""}`;
}

/**
 * Owns durable conversation projections and conversation inspection loads.
 *
 * Session execution can append authoritative messages through
 * `applySessionMessage`, but it does not own conversation selection or
 * inspection loading. This keeps the conversation store reusable by any UI.
 */
export function useConversationStore({ setApiError, setApprovals }) {
  const [conversations, setConversations] = useState([]);
  const [activeConvId, setActiveConvId] = useState(null);
  const [selectedNodeId, setSelectedNodeId] = useState(null);
  const [viewHeadId, setViewHeadId] = useState(null);
  const [inputTokenCounts, setInputTokenCounts] = useState({});

  const selectedNodeRef = useRef(null);
  const loadSeqRef = useRef({});
  const inputTokenSupportRef = useRef({});

  useEffect(() => {
    selectedNodeRef.current = selectedNodeId;
  }, [selectedNodeId]);

  const activeConv = useMemo(
    () => conversations.find((conversation) => conversation.id === activeConvId) || null,
    [activeConvId, conversations]
  );

  const refreshConversations = useCallback(async () => {
    const body = await apiRequest("/api/conversations");
    const summaries = body.conversations.map(conversationSummaryFromApi);
    setConversations((previous) =>
      summaries.map((summary) => {
        const existing = previous.find((conversation) => conversation.id === summary.id);
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

  const applySessionMessage = useCallback((session, message, updatePath) => {
    if (!session?.conversationId || !message?.id) return;
    setConversations((current) =>
      current.map((conversation) => {
        if (conversation.id !== session.conversationId) return conversation;
        return upsertConversationMessage(conversation, message, session.model, updatePath);
      })
    );
  }, []);

  const updateConversation = useCallback((conversationId, update) => {
    setConversations((current) =>
      current.map((conversation) =>
        conversation.id === conversationId ? update(conversation) : conversation
      )
    );
  }, []);

  const loadConversation = useCallback(
    async (convId, options = {}) => {
      if (!convId) return null;
      const hasHead = Object.prototype.hasOwnProperty.call(options, "headMessageId");
      const headMessageId = hasHead ? options.headMessageId : selectedNodeRef.current;
      const query = headMessageId
        ? `?head_message_id=${encodeURIComponent(headMessageId)}`
        : "";
      const seq = (loadSeqRef.current[convId] || 0) + 1;
      loadSeqRef.current[convId] = seq;
      const [report, approvalBody] = await Promise.all([
        apiRequest(`/api/conversations/${convId}${query}`),
        apiRequest(`/api/conversations/${convId}/run-approvals`),
      ]);
      if (loadSeqRef.current[convId] !== seq) return null;

      const loaded = conversationFromInspection(report, null);
      setConversations((previous) => {
        const fallback = previous.find((conversation) => conversation.id === convId);
        const withFallback = conversationFromInspection(report, fallback);
        return previous.some((conversation) => conversation.id === convId)
          ? previous.map((conversation) =>
              conversation.id === convId ? withFallback : conversation
            )
          : [withFallback, ...previous];
      });
      const last = loaded?.selectedPath?.[loaded.selectedPath.length - 1] || loaded?.rootId || null;
      setSelectedNodeId((current) => (current && loaded?.nodes?.[current] ? current : last));
      setApprovals(approvalBody.approvals || []);

      if (options.countTokens !== false && loaded?.id) {
        const model = loaded.model || null;
        const signature = contextSignatureParts(loaded, model).fullSignature;
        const key = tokenCountKey(loaded.id, model);
        if (model && inputTokenSupportRef.current[model] === "unsupported") {
          setInputTokenCounts((previous) => ({
            ...previous,
            [key]: {
              inputTokens: null,
              totalTokens: null,
              model,
              raw: null,
              source: "unsupported",
              signature,
              measuredAt: Date.now(),
            },
          }));
          return loaded;
        }

        const requestId = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
        setInputTokenCounts((previous) => ({
          ...previous,
          [key]: { ...(previous[key] || {}), latestRequestId: requestId },
        }));
        countConversationInputTokens(loaded.id, null, headMessageId || null)
          .then((count) => {
            if (model && count.source === "unsupported") {
              inputTokenSupportRef.current[model] = "unsupported";
            } else if (model && count.inputTokens != null) {
              inputTokenSupportRef.current[model] = "supported";
            }
            setInputTokenCounts((previous) => {
              if (previous[key]?.latestRequestId !== requestId) return previous;
              return {
                ...previous,
                [key]: {
                  ...count,
                  source: count.source || "prequery_input",
                  signature,
                  latestRequestId: requestId,
                  measuredAt: Date.now(),
                },
              };
            });
          })
          .catch(() => {
            setInputTokenCounts((previous) => {
              if (previous[key]?.latestRequestId !== requestId) return previous;
              return {
                ...previous,
                [key]: {
                  inputTokens: null,
                  totalTokens: null,
                  model,
                  raw: null,
                  source: "unavailable",
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
    [setApprovals]
  );

  const selectConversation = useCallback((convId) => {
    setViewHeadId(null);
    setSelectedNodeId(null);
    setActiveConvId(convId);
  }, []);

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
  }, [refreshConversations, setApiError]);

  return {
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
  };
}
