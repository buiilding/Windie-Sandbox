import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  approveSessionTool as approveSessionToolApi,
  continueConversation as continueConversationApi,
  deleteSession as deleteSessionApi,
  denySessionTool as denySessionToolApi,
  getSession,
  listConversationSessions,
  listSessions,
  queryConversation as queryConversationApi,
  resolveSessionAtHead as resolveSessionAtHeadApi,
  stopSession as stopSessionApi,
} from "@/lib/windieApi";
import { streamSessionEvents } from "@/lib/sessionStream";
import { nextSessionEventCursor } from "@/lib/sessionEventCursor";
import { currentSessionHead } from "@/lib/sessionTarget";
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

const SELECTED_SESSION_STORAGE_PREFIX = "windie.selected-session:";

function selectedSessionStorageKey(conversationId) {
  return `${SELECTED_SESSION_STORAGE_PREFIX}${conversationId || ""}`;
}

function readSelectedSessionId(conversationId) {
  if (!conversationId) return null;
  try {
    return window.localStorage.getItem(selectedSessionStorageKey(conversationId));
  } catch (_) {
    return null;
  }
}

function writeSelectedSessionId(conversationId, sessionId) {
  if (!conversationId || !sessionId) return;
  try {
    window.localStorage.setItem(selectedSessionStorageKey(conversationId), sessionId);
  } catch (_) {
    // Browser storage is optional; in-memory selection still works.
  }
}

function emptyPending(session) {
  return {
    convId: session.conversationId,
    text: "",
    reasoning: "",
    toolCalls: {},
    toolCount: 0,
    replaceReasoningOnNextDelta: false,
    replaceTextOnNextDelta: false,
  };
}

function reducePending(current, session, event) {
  const pending = current[session.id] || emptyPending(session);
  if (event.type === "assistant_delta") {
    const text = pending.replaceTextOnNextDelta
      ? event.text || ""
      : pending.text + (event.text || "");
    return {
      ...current,
      [session.id]: {
        ...pending,
        text,
        replaceTextOnNextDelta: false,
      },
    };
  }
  if (event.type === "reasoning_delta") {
    const reasoning = pending.replaceReasoningOnNextDelta
      ? event.text || ""
      : (pending.reasoning || "") + (event.text || "");
    return {
      ...current,
      [session.id]: {
        ...pending,
        reasoning,
        replaceReasoningOnNextDelta: false,
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
    const isNewToolCall = !pending.toolCalls?.[index];
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
        toolCount: (pending.toolCount || 0) + (isNewToolCall ? 1 : 0),
      },
    };
  }
  return current;
}

function resetPendingTurn(pending) {
  return {
    ...pending,
    toolCalls: {},
    replaceReasoningOnNextDelta: true,
    replaceTextOnNextDelta: true,
  };
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
  viewHeadId,
  setViewHeadId,
  selectedNodeId,
  setSelectedNodeId,
  loadConversation,
  applySessionMessage,
  setApiError,
}) {
  const [sessionsById, setSessionsById] = useState({});
  const [selectedSessionId, setSelectedSessionId] = useState(null);
  const [pendingAssistantBySessionId, setPendingAssistantBySessionId] = useState({});
  const [sessionResolution, setSessionResolution] = useState({
    status: "idle",
    kind: null,
    error: null,
  });

  // These refs represent runtime resources, not alternate application state:
  // open SSE controllers, replay cursors, and the latest selected session for
  // async callbacks that outlive a render.
  const subscriptionsRef = useRef(new Map());
  const lastEventIdRef = useRef({});
  const sessionsRef = useRef({});
  const selectedSessionRef = useRef(null);
  const conversationIdRef = useRef(conversationId);

  const rememberSession = useCallback((session) => {
    if (!session) return null;
    const existing = sessionsRef.current[session.id];
    const merged = existing?.latestEventId != null &&
      (session.latestEventId == null || session.latestEventId < existing.latestEventId)
      ? { ...session, latestEventId: existing.latestEventId }
      : session;
    setSessionsById((current) => ({ ...current, [merged.id]: merged }));
    sessionsRef.current = { ...sessionsRef.current, [merged.id]: merged };
    if (selectedSessionRef.current?.id === merged.id) {
      selectedSessionRef.current = merged;
    }
    return merged;
  }, []);

  const hydrateSessionEventCursor = useCallback((session) => {
    if (!session || session.latestEventId == null) return;
    const current = lastEventIdRef.current[session.id];
    if (current == null || session.latestEventId > current) {
      lastEventIdRef.current[session.id] = session.latestEventId;
    }
  }, []);

  const advanceSessionEventCursor = useCallback((sessionId, eventId) => {
    if (!sessionId) return false;
    const currentCursor = lastEventIdRef.current[sessionId];
    const next = nextSessionEventCursor(currentCursor, eventId);
    if (!next.accepted) return false;
    if (next.cursor != null) lastEventIdRef.current[sessionId] = next.cursor;

    const current = sessionsRef.current[sessionId];
    if (!current) return true;
    if (next.cursor == null) return true;
    const updated = { ...current, latestEventId: next.cursor };
    sessionsRef.current = { ...sessionsRef.current, [sessionId]: updated };
    if (selectedSessionRef.current?.id === sessionId) {
      selectedSessionRef.current = updated;
    }
    setSessionsById((sessions) => ({ ...sessions, [sessionId]: updated }));
    return true;
  }, []);

  const abortAllSubscriptions = useCallback(() => {
    for (const controller of subscriptionsRef.current.values()) controller.abort();
    subscriptionsRef.current.clear();
  }, []);

  useEffect(() => {
    conversationIdRef.current = conversationId;
  }, [conversationId]);

  useEffect(
    () => () => abortAllSubscriptions(),
    [abortAllSubscriptions]
  );

  const handleEvent = useCallback(
    async (session, data) => {
      if (!data?.type) return;
      const snapshot = data.session ? sessionFromApi(data.session) : null;
      const currentSession = snapshot || session;
      if (snapshot) rememberSession(snapshot);
      const selected =
        currentSession.conversationId === conversationIdRef.current &&
        selectedSessionRef.current?.id === currentSession.id;

      if (data.type === "input_queued") {
        if (!snapshot) {
          rememberSession({
            ...currentSession,
            queueDepth: data.queue_depth ?? currentSession.queueDepth ?? 0,
          });
        }
        return;
      }
      if (data.type === "input_started") {
        if (data.message) {
          applySessionMessage(currentSession, data.message, selected);
        }
        return;
      }
      if (["assistant_delta", "reasoning_delta", "tool_call_delta"].includes(data.type)) {
        setPendingAssistantBySessionId((current) => reducePending(current, session, data));
        return;
      }

      if (data.type === "assistant_message_saved" || data.type === "tool_result_saved") {
        if (data.message) {
          applySessionMessage(currentSession, data.message, selected);
        }
        setPendingAssistantBySessionId((current) => {
          const pending = current[currentSession.id];
          if (!pending) return current;

          if (data.type === "tool_result_saved" || Object.keys(pending.toolCalls || {}).length > 0) {
            return { ...current, [currentSession.id]: resetPendingTurn(pending) };
          }

          return { ...current, [currentSession.id]: null };
        });
        return;
      }

      if (!["completed", "failed", "cancelled", "waiting_for_approval"].includes(data.type)) {
        return;
      }

      const latest = snapshot ? null : await getSession(currentSession.id).catch(() => null);
      const normalized = latest ? sessionFromApi(latest) : currentSession;

      if (
        currentSession.conversationId !== conversationIdRef.current ||
        selectedSessionRef.current?.id !== currentSession.id
      ) {
        if (normalized) rememberSession(normalized);
        setPendingAssistantBySessionId((current) => ({ ...current, [currentSession.id]: null }));
        return;
      }

      if (normalized) rememberSession(normalized);
      setPendingAssistantBySessionId((current) => ({ ...current, [currentSession.id]: null }));
    },
    [applySessionMessage, rememberSession]
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
          const eventId = id ?? data?.event_id ?? null;
          const accepted = advanceSessionEventCursor(normalized.id, eventId);
          if (eventId != null && !accepted) return;
          const current = sessionsRef.current[normalized.id] || normalized;
          await handleEvent(current, data);
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
    [advanceSessionEventCursor, handleEvent, rememberSession, setApiError]
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
    sessions.forEach(hydrateSessionEventCursor);
    const next = Object.fromEntries(sessions.map((session) => [session.id, session]));
    setSessionsById(next);
    sessionsRef.current = next;
    const selectedId = selectedSessionRef.current?.id;
    selectedSessionRef.current = selectedId ? next[selectedId] || null : null;
    reconcileSubscriptions();
    return sessions;
  }, [hydrateSessionEventCursor, reconcileSubscriptions]);

  useEffect(() => {
    refreshSessions().catch((error) => setApiError(error.message));
  }, [refreshSessions, setApiError]);

  useEffect(() => {
    if (!conversationId) {
      setSessionResolution({ status: "idle", kind: null, error: null });
      setViewHeadId(null);
      setSelectedSessionId(null);
      selectedSessionRef.current = null;
      return undefined;
    }

    let cancelled = false;
    setSessionResolution({ status: "idle", kind: null, error: null });
    (async () => {
      const sessions = (await listConversationSessions(conversationId))
        .map(sessionFromApi)
        .filter(Boolean);
      sessions.forEach(hydrateSessionEventCursor);
      const byId = Object.fromEntries(sessions.map((session) => [session.id, session]));
      setSessionsById((current) => ({ ...current, ...byId }));
      sessionsRef.current = { ...sessionsRef.current, ...byId };

      const rememberedId = readSelectedSessionId(conversationId);
      const remembered = rememberedId
        ? sessions.find((session) => session.id === rememberedId)
        : null;
      const selected = remembered || sessions.find(isLiveSession) || sessions[0] || null;
      setSelectedSessionId(selected?.id || null);
      selectedSessionRef.current = selected;
      if (selected) writeSelectedSessionId(conversationId, selected.id);
      await loadConversation(conversationId, {
        headMessageId: currentSessionHead(selected),
      });
      if (!cancelled) setApiError(null);
    })().catch((error) => {
      if (!cancelled) setApiError(error.message);
    });

    return () => {
      cancelled = true;
    };
  }, [conversationId, hydrateSessionEventCursor, loadConversation, setApiError, setViewHeadId]);

  const selectedSession = useMemo(
    () => (selectedSessionId ? sessionsById[selectedSessionId] || null : null),
    [selectedSessionId, sessionsById]
  );

  const selectSession = useCallback(
    async (sessionId, suppliedSession = null) => {
      const session = suppliedSession || sessionsRef.current[sessionId];
      if (!session || session.conversationId !== conversationId) return null;
      rememberSession(session);
      setViewHeadId(null);
      setSelectedSessionId(session.id);
      selectedSessionRef.current = session;
      hydrateSessionEventCursor(session);
      writeSelectedSessionId(conversationId, session.id);
      const head = currentSessionHead(session);
      setSelectedNodeId(head);
      await loadConversation(conversationId, {
        headMessageId: head,
        countTokens: false,
      });
      if (isLiveSession(session)) subscribeToSession(session);
      return session;
    },
    [conversationId, hydrateSessionEventCursor, loadConversation, rememberSession, setSelectedNodeId, setViewHeadId, subscribeToSession]
  );

  const resolvePathHead = useCallback(
    async (headMessageId) => {
      if (!conversationId) return { kind: "none" };
      setSessionResolution({ status: "resolving", kind: null, error: null });
      try {
        const response = await resolveSessionAtHeadApi(conversationId, headMessageId);
        if (response.type === "existing_session") {
          const session = sessionFromApi(response.session);
          await selectSession(session.id, session);
          setSessionResolution({ status: "resolved", kind: "existing", error: null });
          return { kind: "existing", session };
        }
        if (response.type === "no_session_at_head") {
          setSessionResolution({ status: "resolved", kind: "none", error: null });
          return { kind: "none" };
        }

        const message = "multiple sessions exist at this conversation head";
        setSessionResolution({ status: "error", kind: "ambiguous", error: message });
        throw new Error(message);
      } catch (error) {
        setSessionResolution({ status: "error", kind: "error", error: error.message });
        throw error;
      }
    },
    [conversationId, selectSession]
  );

  const sendMessage = useCallback(
    async (text, options = {}) => {
      if (!conversationId) return;
      const attachments = options.attachments || [];
      if (!text.trim() && attachments.length === 0) return;

      try {
        const parts = await messagePartsForSend(text, attachments);
        const parentHead = viewHeadId || currentSessionHead(selectedSessionRef.current) || selectedNodeId || null;
        const updated = sessionFromApi(await queryConversationApi(conversationId, {
          headMessageId: parentHead,
          parts,
        }));
        rememberSession(updated);
        setSelectedSessionId(updated.id);
        selectedSessionRef.current = updated;
        writeSelectedSessionId(conversationId, updated.id);
        if (!updated.queued) {
          setPendingAssistantBySessionId((current) => ({
            ...current,
            [updated.id]: emptyPending(updated),
          }));
          await loadConversation(conversationId, {
            headMessageId: updated.currentHeadMessageId,
            countTokens: false,
          });
        } else {
          toast.message("message queued", {
            description: `${updated.queueDepth} message${updated.queueDepth === 1 ? "" : "s"} waiting`,
          });
        }
        subscribeToSession(updated);
        setViewHeadId(null);
        setApiError(null);
      } catch (error) {
        setApiError(error.message);
        toast.error(error.message);
      }
    },
    [conversationId, loadConversation, selectedNodeId, setApiError, rememberSession, setViewHeadId, subscribeToSession, viewHeadId]
  );

  const continueConversation = useCallback(async () => {
    try {
      const headMessageId = viewHeadId || currentSessionHead(selectedSessionRef.current) || selectedNodeId || null;
      const session = sessionFromApi(await continueConversationApi(conversationId, headMessageId));
      rememberSession(session);
      selectedSessionRef.current = session;
      setSelectedSessionId(session.id);
      setViewHeadId(null);
      setPendingAssistantBySessionId((current) => ({
        ...current,
        [session.id]: emptyPending(session),
      }));
      subscribeToSession(session);
      setApiError(null);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [conversationId, selectedNodeId, rememberSession, setApiError, setViewHeadId, subscribeToSession, viewHeadId]);

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
          if (replacement) writeSelectedSessionId(conversationId, replacement.id);
          setViewHeadId(null);
          const head =
            currentSessionHead(replacement);
          setSelectedNodeId(head);
          await loadConversation(conversationId, {
            headMessageId: head,
            countTokens: false,
          });
        } else if (removed?.conversationId === conversationId) {
          await loadConversation(conversationId, {
            headMessageId:
              viewHeadId ||
              currentSessionHead(selectedSessionRef.current) ||
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

  const getSelectedSession = useCallback(() => selectedSessionRef.current, []);

  return {
    sessionsById,
    selectedSession,
    selectedSessionId,
    getSelectedSession,
    resolvePathHead,
    sessionResolution,
    selectedPathHead:
      viewHeadId ||
      currentSessionHead(selectedSession) ||
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
