import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  approveSessionTool as approveSessionToolApi,
  continueSession as continueSessionApi,
  createSession as createSessionApi,
  deleteSession as deleteSessionApi,
  denySessionTool as denySessionToolApi,
  getSession,
  listConversationSessions,
  listSessions,
  querySession as querySessionApi,
  stopSession as stopSessionApi,
} from "@/lib/windieApi";
import { streamSessionEvents } from "@/lib/sessionStream";
import { sessionFromApi } from "@/lib/windieMappers";

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
  if (text.trim()) parts.push({ type: "text", text });
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

function isLiveSession(session) {
  return session?.status === "running" || session?.status === "waiting_for_approval";
}

function isAbortError(error) {
  return error?.name === "AbortError";
}

function emptyPending(session) {
  return {
    convId: session.conversationId,
    text: "",
    reasoning: "",
    toolCalls: {},
  };
}

function reducePending(current, session, event) {
  const pending = current[session.id] || emptyPending(session);
  if (event.type === "assistant_delta") {
    return {
      ...current,
      [session.id]: { ...pending, text: pending.text + (event.text || "") },
    };
  }
  if (event.type === "reasoning_delta") {
    return {
      ...current,
      [session.id]: {
        ...pending,
        reasoning: (pending.reasoning || "") + (event.text || ""),
      },
    };
  }
  if (event.type === "tool_call_delta") {
    const index = String(event.index ?? 0);
    const existing = pending.toolCalls?.[index] || {
      id: null,
      name: null,
      argumentsText: "",
    };
    return {
      ...current,
      [session.id]: {
        ...pending,
        toolCalls: {
          ...(pending.toolCalls || {}),
          [index]: {
            id: event.id || existing.id,
            name: event.name || existing.name,
            argumentsText: existing.argumentsText + (event.arguments_delta || ""),
          },
        },
      },
    };
  }
  return current;
}

function optimisticParts(conversationId, parts) {
  return parts.map((part) => {
    if (part.type === "text") return { type: "text", text: part.text || "" };
    return {
      type: "image",
      alt: part.path || "attachment",
      assetId: null,
      conversationId,
      mimeType: part.mime_type || null,
      byteCount: null,
    };
  });
}

function addOptimisticUserMessage(setConversations, conversationId, messageId, parentId, parts) {
  if (!conversationId || !messageId) return;
  const messageParts = optimisticParts(conversationId, parts);

  setConversations((current) => current.map((conversation) => {
    if (conversation.id !== conversationId || conversation.nodes?.[messageId]) return conversation;

    const node = {
      id: messageId,
      parentId: parentId || null,
      childrenIds: [],
      message: {
        role: "user",
        parts: messageParts,
        metadata: null,
        timestamp: new Date().toISOString(),
      },
    };
    const nodes = { ...(conversation.nodes || {}), [messageId]: node };
    if (parentId && nodes[parentId]) {
      nodes[parentId] = {
        ...nodes[parentId],
        childrenIds: Array.from(new Set([...(nodes[parentId].childrenIds || []), messageId])),
      };
    }

    const rootIds = parentId
      ? (conversation.rootIds || (conversation.rootId ? [conversation.rootId] : []))
      : Array.from(new Set([...(conversation.rootIds || []), messageId]));

    return {
      ...conversation,
      nodes,
      rootIds,
      rootId: conversation.rootId || (!parentId ? messageId : null),
      messageCount: Object.keys(nodes).length,
    };
  }));
}

/**
 * Owns durable session selection and live session execution.
 *
 * A session is the only runtime target. Conversation loading is deliberately
 * kept as an injected operation: this hook tells the conversation store which
 * head to load, then reduces session events into a transient stream preview.
 */
export function useSessionRuntime({
  conversationId,
  conversationModel,
  reasoning,
  viewHeadId,
  setViewHeadId,
  selectedNodeId,
  setSelectedNodeId,
  setConversations,
  loadConversation,
  setApiError,
}) {
  const [sessionsById, setSessionsById] = useState({});
  const [selectedSessionId, setSelectedSessionId] = useState(null);
  const [pendingAssistantBySessionId, setPendingAssistantBySessionId] = useState({});

  // These refs represent runtime resources, not alternate application state:
  // open SSE controllers, replay cursors, and the latest selected session for
  // async callbacks that outlive a render.
  const subscriptionsRef = useRef(new Map());
  const lastEventIdRef = useRef({});
  const sessionsRef = useRef({});
  const selectedSessionRef = useRef(null);
  const conversationIdRef = useRef(conversationId);
  const renderedHeadRef = useRef({});

  const rememberSession = useCallback((session) => {
    if (!session) return null;
    setSessionsById((current) => ({ ...current, [session.id]: session }));
    sessionsRef.current = { ...sessionsRef.current, [session.id]: session };
    if (selectedSessionRef.current?.id === session.id) {
      selectedSessionRef.current = session;
    }
    return session;
  }, []);

  const abortAllSubscriptions = useCallback(() => {
    for (const controller of subscriptionsRef.current.values()) controller.abort();
    subscriptionsRef.current.clear();
  }, []);

  const advanceSessionHead = useCallback((sessionId, headMessageId) => {
    if (!sessionId || !headMessageId) return;
    const current = sessionsRef.current[sessionId];
    if (!current) return;
    const updated = { ...current, currentHeadMessageId: headMessageId };
    sessionsRef.current = { ...sessionsRef.current, [sessionId]: updated };
    if (selectedSessionRef.current?.id === sessionId) {
      selectedSessionRef.current = updated;
    }
    setSessionsById((sessions) => ({ ...sessions, [sessionId]: updated }));
  }, []);

  useEffect(() => {
    conversationIdRef.current = conversationId;
  }, [conversationId]);

  useEffect(
    () => () => abortAllSubscriptions(),
    [abortAllSubscriptions]
  );

  const refreshActiveConversation = useCallback(
    async (sessionId, headMessageId) => {
      const activeConversationId = conversationIdRef.current;
      if (!activeConversationId || selectedSessionRef.current?.id !== sessionId) return false;
      const loaded = await loadConversation(activeConversationId, {
        headMessageId: headMessageId || null,
        countTokens: false,
      });
      if (!loaded) return false;
      renderedHeadRef.current[sessionId] = headMessageId || null;
      return true;
    },
    [loadConversation]
  );

  const commitMessage = useCallback(
    async (session, headMessageId) => {
      if (!headMessageId) return false;
      const active =
        session.conversationId === conversationIdRef.current &&
        selectedSessionRef.current?.id === session.id;
      if (!active) {
        advanceSessionHead(session.id, headMessageId);
        setPendingAssistantBySessionId((current) => ({ ...current, [session.id]: null }));
        return true;
      }

      try {
        const reloaded = await refreshActiveConversation(session.id, headMessageId);
        if (!reloaded) return false;
        advanceSessionHead(session.id, headMessageId);
        setSelectedNodeId(headMessageId);
        setPendingAssistantBySessionId((current) => ({ ...current, [session.id]: null }));
        return true;
      } catch (_) {
        return false;
      }
    },
    [advanceSessionHead, refreshActiveConversation, setSelectedNodeId]
  );

  const handleEvent = useCallback(
    async (session, data) => {
      if (!data?.type) return;
      if (["assistant_delta", "reasoning_delta", "tool_call_delta"].includes(data.type)) {
        setPendingAssistantBySessionId((current) => reducePending(current, session, data));
        return;
      }

      if (data.type === "assistant_message_saved" || data.type === "tool_result_saved") {
        const head = data.message_id || null;
        if (!head) return;
        await commitMessage(session, head);
        return;
      }

      if (!["completed", "failed", "cancelled", "waiting_for_approval"].includes(data.type)) {
        return;
      }

      const latest = await getSession(session.id).catch(() => null);
      const normalized = latest ? sessionFromApi(latest) : null;
      const head = data.message_id || normalized?.currentHeadMessageId || normalized?.startHeadMessageId || null;

      if (
        session.conversationId !== conversationIdRef.current ||
        selectedSessionRef.current?.id !== session.id
      ) {
        if (normalized) rememberSession(normalized);
        setPendingAssistantBySessionId((current) => ({ ...current, [session.id]: null }));
        return;
      }

      try {
        let committed = renderedHeadRef.current[session.id] === head;
        if (renderedHeadRef.current[session.id] !== head) {
          committed = await commitMessage(session, head);
        }
        if (!committed) return;
        if (normalized) rememberSession(normalized);
        setPendingAssistantBySessionId((current) => ({ ...current, [session.id]: null }));
      } catch (_) {
        // Keep the preview if the authoritative tree reload failed.
      }
    },
    [commitMessage, rememberSession]
  );

  const subscribeToSession = useCallback(
    (session) => {
      const normalized = rememberSession(session);
      if (!normalized) return null;
      if (subscriptionsRef.current.has(normalized.id)) return normalized;

      const controller = new AbortController();
      subscriptionsRef.current.set(normalized.id, controller);
      streamSessionEvents(
        normalized.id,
        lastEventIdRef.current[normalized.id] ?? null,
        async ({ id, data }) => {
          await handleEvent(normalized, data);
          const eventId = id ?? data?.event_id ?? null;
          if (eventId != null) lastEventIdRef.current[normalized.id] = eventId;
        },
        { signal: controller.signal }
      )
        .catch((error) => {
          if (!isAbortError(error)) {
            setApiError(error.message);
            toast.error(error.message);
          }
        })
        .finally(() => {
          if (subscriptionsRef.current.get(normalized.id) === controller) {
            subscriptionsRef.current.delete(normalized.id);
          }
        });
      return normalized;
    },
    [handleEvent, rememberSession, setApiError]
  );

  const reconcileSubscriptions = useCallback(() => {
    const liveSessions = Object.values(sessionsRef.current).filter(isLiveSession);
    const liveIds = new Set(liveSessions.map((session) => session.id));

    for (const session of liveSessions) subscribeToSession(session);
    for (const [sessionId, controller] of Array.from(subscriptionsRef.current.entries())) {
      if (liveIds.has(sessionId)) continue;
      controller.abort();
      subscriptionsRef.current.delete(sessionId);
    }
  }, [subscribeToSession]);

  useEffect(() => {
    reconcileSubscriptions();
  }, [reconcileSubscriptions, sessionsById]);

  const refreshSessions = useCallback(async () => {
    const sessions = (await listSessions()).map(sessionFromApi).filter(Boolean);
    const next = Object.fromEntries(sessions.map((session) => [session.id, session]));
    setSessionsById(next);
    sessionsRef.current = next;
    const selectedId = selectedSessionRef.current?.id;
    selectedSessionRef.current = selectedId ? next[selectedId] || null : null;
    reconcileSubscriptions();
    return sessions;
  }, [reconcileSubscriptions]);

  useEffect(() => {
    refreshSessions().catch((error) => setApiError(error.message));
  }, [refreshSessions, setApiError]);

  useEffect(() => {
    if (!conversationId) {
      setViewHeadId(null);
      setSelectedSessionId(null);
      selectedSessionRef.current = null;
      return undefined;
    }

    let cancelled = false;
    (async () => {
      const sessions = (await listConversationSessions(conversationId))
        .map(sessionFromApi)
        .filter(Boolean);
      const byId = Object.fromEntries(sessions.map((session) => [session.id, session]));
      setSessionsById((current) => ({ ...current, ...byId }));
      sessionsRef.current = { ...sessionsRef.current, ...byId };

      const selected = sessions.find(isLiveSession) || sessions[0] || null;
      setSelectedSessionId(selected?.id || null);
      selectedSessionRef.current = selected;
      await loadConversation(conversationId, {
        headMessageId: selected?.currentHeadMessageId || selected?.startHeadMessageId || null,
      });
      if (!cancelled) setApiError(null);
    })().catch((error) => {
      if (!cancelled) setApiError(error.message);
    });

    return () => {
      cancelled = true;
    };
  }, [conversationId, loadConversation, setApiError, setViewHeadId]);

  const selectedSession = useMemo(
    () => (selectedSessionId ? sessionsById[selectedSessionId] || null : null),
    [selectedSessionId, sessionsById]
  );

  const selectSession = useCallback(
    async (sessionId) => {
      const session = sessionsRef.current[sessionId];
      if (!session || session.conversationId !== conversationId) return null;
      setViewHeadId(null);
      setSelectedSessionId(session.id);
      selectedSessionRef.current = session;
      const head = session.currentHeadMessageId || session.startHeadMessageId || null;
      setSelectedNodeId(head);
      await loadConversation(conversationId, {
        headMessageId: head,
        countTokens: false,
      });
      if (isLiveSession(session)) subscribeToSession(session);
      return session;
    },
    [conversationId, loadConversation, setSelectedNodeId, setViewHeadId, subscribeToSession]
  );

  const sendMessage = useCallback(
    async (text, options = {}) => {
      if (!conversationId) return;
      const attachments = options.attachments || [];
      if (!text.trim() && attachments.length === 0) return;

      try {
        const parts = await messagePartsForSend(text, attachments);
        let session = selectedSessionRef.current;
        const sessionHead = session?.currentHeadMessageId || session?.startHeadMessageId || null;
        const queryHead = viewHeadId || sessionHead || selectedNodeId || null;
        const needsNewSession = viewHeadId && viewHeadId !== sessionHead;
        if (!session || session.conversationId !== conversationId || needsNewSession) {
          session = sessionFromApi(
            await createSessionApi(conversationId, {
              headMessageId: queryHead,
              model: conversationModel || null,
              reasoning,
            })
          );
          rememberSession(session);
          setSelectedSessionId(session.id);
          selectedSessionRef.current = session;
        }
        if (isLiveSession(session)) {
          toast.message("session busy", { description: "wait for this session to finish" });
          return;
        }

        const parentHead = session.currentHeadMessageId || session.startHeadMessageId || null;
        const updated = sessionFromApi(await querySessionApi(session.id, parts));
        rememberSession(updated);
        setSelectedSessionId(updated.id);
        selectedSessionRef.current = updated;
        setSelectedNodeId(updated.currentHeadMessageId);
        addOptimisticUserMessage(setConversations, conversationId, updated.currentHeadMessageId, parentHead, parts);
        subscribeToSession(updated);
        setViewHeadId(null);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [conversationId, conversationModel, reasoning, selectedNodeId, setSelectedNodeId, setConversations, setApiError, rememberSession, setViewHeadId, subscribeToSession, viewHeadId]
  );

  const continueConversation = useCallback(async () => {
    let selected = selectedSessionRef.current;
    const selectedHead = selected?.currentHeadMessageId || selected?.startHeadMessageId || null;
    const needsNewSession = viewHeadId && viewHeadId !== selectedHead;
    try {
      if (!selected || selected.conversationId !== conversationId || needsNewSession) {
        selected = sessionFromApi(
          await createSessionApi(conversationId, {
            headMessageId: viewHeadId || selectedHead || null,
            model: conversationModel || null,
            reasoning,
          })
        );
        rememberSession(selected);
        setSelectedSessionId(selected.id);
        selectedSessionRef.current = selected;
      }
      if (isLiveSession(selected)) return;
      const session = sessionFromApi(await continueSessionApi(selected.id));
      rememberSession(session);
      selectedSessionRef.current = session;
      setSelectedSessionId(session.id);
      setViewHeadId(null);
      subscribeToSession(session);
      setApiError(null);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [conversationId, conversationModel, reasoning, rememberSession, setApiError, setViewHeadId, subscribeToSession, viewHeadId]);

  const approveToolCall = useCallback(async (sessionId, toolCallId) => {
    if (!sessionId) return;
    try {
      const session = sessionFromApi(await approveSessionToolApi(sessionId, toolCallId));
      rememberSession(session);
      subscribeToSession(session);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [rememberSession, setApiError, subscribeToSession]);

  const denyToolCall = useCallback(async (sessionId, toolCallId) => {
    if (!sessionId) return;
    try {
      const session = sessionFromApi(await denySessionToolApi(sessionId, toolCallId));
      rememberSession(session);
      subscribeToSession(session);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [rememberSession, setApiError, subscribeToSession]);

  const stopStreaming = useCallback(async (sessionId = selectedSessionId) => {
    const targetSessionId =
      typeof sessionId === "string" ? sessionId : selectedSessionId;
    if (!targetSessionId) return;
    try {
      const session = sessionFromApi(await stopSessionApi(targetSessionId));
      rememberSession(session);
      setPendingAssistantBySessionId((current) => ({
        ...current,
        [targetSessionId]: null,
      }));
      const controller = subscriptionsRef.current.get(targetSessionId);
      if (controller) {
        controller.abort();
        subscriptionsRef.current.delete(targetSessionId);
      }
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [rememberSession, selectedSessionId, setApiError]);

  const deleteSession = useCallback(
    async (sessionId) => {
      if (!sessionId) return false;
      const removed = sessionsRef.current[sessionId] || null;
      try {
        await deleteSessionApi(sessionId);

        const controller = subscriptionsRef.current.get(sessionId);
        if (controller) {
          controller.abort();
          subscriptionsRef.current.delete(sessionId);
        }
        delete lastEventIdRef.current[sessionId];

        const next = { ...sessionsRef.current };
        delete next[sessionId];
        sessionsRef.current = next;
        setSessionsById(next);
        setPendingAssistantBySessionId((current) => {
          const pending = { ...current };
          delete pending[sessionId];
          return pending;
        });
        reconcileSubscriptions();

        if (selectedSessionRef.current?.id === sessionId) {
          const replacement = Object.values(next)
            .filter((session) => session.conversationId === conversationId)
            .sort(
              (a, b) =>
                (b.updatedAt || b.createdAt || 0) -
                (a.updatedAt || a.createdAt || 0)
            )[0] || null;
          setSelectedSessionId(replacement?.id || null);
          selectedSessionRef.current = replacement;
          setViewHeadId(null);
          const head =
            replacement?.currentHeadMessageId ||
            replacement?.startHeadMessageId ||
            null;
          setSelectedNodeId(head);
          await loadConversation(conversationId, {
            headMessageId: head,
            countTokens: false,
          });
        } else if (removed?.conversationId === conversationId) {
          await loadConversation(conversationId, {
            headMessageId:
              viewHeadId ||
              selectedSessionRef.current?.currentHeadMessageId ||
              selectedSessionRef.current?.startHeadMessageId ||
              null,
            countTokens: false,
          });
        }

        toast.message("session deleted");
        return true;
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
        return false;
      }
    },
    [
      conversationId,
      loadConversation,
      reconcileSubscriptions,
      setApiError,
      setSelectedNodeId,
      setViewHeadId,
      viewHeadId,
    ]
  );

  return {
    sessionsById,
    selectedSession,
    selectedSessionId,
    selectedPathHead:
      viewHeadId ||
      selectedSession?.currentHeadMessageId ||
      selectedSession?.startHeadMessageId ||
      selectedNodeId ||
      null,
    pendingAssistant: selectedSessionId ? pendingAssistantBySessionId[selectedSessionId] || null : null,
    streaming: isLiveSession(selectedSession),
    refreshSessions,
    selectSession,
    sendMessage,
    continueConversation,
    stopStreaming,
    deleteSession,
    approveToolCall,
    denyToolCall,
  };
}
