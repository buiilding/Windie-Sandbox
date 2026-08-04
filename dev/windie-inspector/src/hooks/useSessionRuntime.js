import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  approveSessionTool as approveSessionToolApi,
  continueConversation as continueConversationApi,
  deleteSession as deleteSessionApi,
  denySessionTool as denySessionToolApi,
  listConversationSessions,
  listSessions,
  queryConversation as queryConversationApi,
  resolveSessionAtHead as resolveSessionAtHeadApi,
  stopSession as stopSessionApi,
} from "@/lib/windieApi";
import { currentSessionHead } from "@/lib/sessionTarget";
import { projectSessionEvent } from "@/lib/sessionEvent";
import { sessionFromApi } from "@/lib/windieMappers";
import { messagePartsForSend } from "@/lib/sessionInput";
import {
  isLiveSession,
  readSelectedSessionId,
  writeSelectedSessionId,
} from "@/lib/sessionState";
import { useSessionPreview } from "@/hooks/useSessionPreview";
import { useSessionStore } from "@/hooks/useSessionStore";
import { useSessionTransport } from "@/hooks/useSessionTransport";

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
  const [selectedSessionId, setSelectedSessionId] = useState(null);
  const [sessionResolution, setSessionResolution] = useState({
    status: "idle",
    kind: null,
    error: null,
  });

  const {
    pendingBySessionId,
    startTurn,
    applyDelta,
    applySavedMessage,
    clearSession: clearPreview,
    removeSession: removePreview,
  } = useSessionPreview();

  const {
    sessionsById,
    sessionsRef,
    rememberSession: rememberStoredSession,
    rememberSessions,
    replaceSessions,
    removeSession: removeStoredSession,
  } = useSessionStore();
  const selectedSessionRef = useRef(null);
  const conversationIdRef = useRef(conversationId);

  const rememberSession = useCallback((session) => {
    const merged = rememberStoredSession(session);
    if (merged && selectedSessionRef.current?.id === merged.id) {
      selectedSessionRef.current = merged;
    }
    return merged;
  }, [rememberStoredSession]);

  useEffect(() => {
    conversationIdRef.current = conversationId;
  }, [conversationId]);

  const handleEvent = useCallback(
    (session, data) => {
      const projection = projectSessionEvent(session, data);
      if (!projection) return;
      const currentSession = projection.session;
      if (data.session) rememberSession(currentSession);
      const selected =
        currentSession.conversationId === conversationIdRef.current &&
        selectedSessionRef.current?.id === currentSession.id;

      if (projection.type === "input_queued") return;
      if (projection.type === "input_started") {
        if (projection.message) {
          applySessionMessage(currentSession, projection.message, selected);
        }
        return;
      }
      if (projection.isDelta) {
        applyDelta(currentSession, data);
        return;
      }

      if (projection.isSavedMessage) {
        if (projection.message) {
          applySessionMessage(currentSession, projection.message, selected);
        }
        applySavedMessage(currentSession, projection.type);
        return;
      }

      if (projection.type === "waiting_for_approval" && selected) {
        loadConversation(currentSession.conversationId, {
          headMessageId: currentSession.currentHeadMessageId,
          countTokens: false,
        }).catch((error) => setApiError(error.message));
      }

      if (!projection.isTerminal) return;

      if (
        currentSession.conversationId !== conversationIdRef.current ||
        selectedSessionRef.current?.id !== currentSession.id
      ) {
        clearPreview(currentSession.id);
        return;
      }

      clearPreview(currentSession.id);
    },
    [applyDelta, applySavedMessage, applySessionMessage, clearPreview, loadConversation, rememberSession, setApiError]
  );

  const onTransportError = useCallback((error) => {
    setApiError(error.message);
    toast.error(error.message);
  }, [setApiError]);

  const {
    hydrateSessionEventCursor,
    subscribe: subscribeToSession,
    reconcileSubscriptions,
    clearSession: clearTransportSession,
  } = useSessionTransport({
    sessionsRef,
    rememberSession,
    onEvent: handleEvent,
    onError: onTransportError,
  });

  useEffect(() => {
    reconcileSubscriptions();
  }, [reconcileSubscriptions, sessionsById]);

  const refreshSessions = useCallback(async () => {
    const sessions = (await listSessions()).map(sessionFromApi).filter(Boolean);
    sessions.forEach(hydrateSessionEventCursor);
    const next = replaceSessions(sessions);
    const selectedId = selectedSessionRef.current?.id;
    selectedSessionRef.current = selectedId ? next[selectedId] || null : null;
    reconcileSubscriptions();
    return sessions;
  }, [hydrateSessionEventCursor, reconcileSubscriptions, replaceSessions]);

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
      rememberSessions(Object.values(byId));

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
  }, [conversationId, hydrateSessionEventCursor, loadConversation, rememberSessions, setApiError, setViewHeadId]);

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
    [conversationId, hydrateSessionEventCursor, loadConversation, rememberSession, sessionsRef, setSelectedNodeId, setViewHeadId, subscribeToSession]
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
          startTurn(updated);
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
    [conversationId, loadConversation, selectedNodeId, setApiError, rememberSession, setViewHeadId, startTurn, subscribeToSession, viewHeadId]
  );

  const continueConversation = useCallback(async () => {
    try {
      const headMessageId = viewHeadId || currentSessionHead(selectedSessionRef.current) || selectedNodeId || null;
      const session = sessionFromApi(await continueConversationApi(conversationId, headMessageId));
      rememberSession(session);
      selectedSessionRef.current = session;
      setSelectedSessionId(session.id);
      setViewHeadId(null);
      startTurn(session);
      subscribeToSession(session);
      setApiError(null);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [conversationId, selectedNodeId, rememberSession, setApiError, setViewHeadId, startTurn, subscribeToSession, viewHeadId]);

  const approveToolCall = useCallback(async (sessionId, toolCallId) => {
    if (!sessionId) return;
    try {
      const session = sessionFromApi(await approveSessionToolApi(sessionId, toolCallId));
      rememberSession(session);
      await loadConversation(session.conversationId, {
        headMessageId: session.currentHeadMessageId,
        countTokens: false,
      });
      subscribeToSession(session);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [loadConversation, rememberSession, setApiError, subscribeToSession]);

  const denyToolCall = useCallback(async (sessionId, toolCallId) => {
    if (!sessionId) return;
    try {
      const session = sessionFromApi(await denySessionToolApi(sessionId, toolCallId));
      rememberSession(session);
      await loadConversation(session.conversationId, {
        headMessageId: session.currentHeadMessageId,
        countTokens: false,
      });
      subscribeToSession(session);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [loadConversation, rememberSession, setApiError, subscribeToSession]);

  const stopStreaming = useCallback(async (sessionId = selectedSessionId) => {
    const targetSessionId =
      typeof sessionId === "string" ? sessionId : selectedSessionId;
    if (!targetSessionId) return;
    try {
      const session = sessionFromApi(await stopSessionApi(targetSessionId));
      rememberSession(session);
      clearPreview(targetSessionId);
      clearTransportSession(targetSessionId);
    } catch (error) {
      setApiError(error.message);
      toast.error(error.message);
    }
  }, [clearPreview, clearTransportSession, rememberSession, selectedSessionId, setApiError]);

  const deleteSession = useCallback(
    async (sessionId) => {
      if (!sessionId) return false;
      const removed = sessionsRef.current[sessionId] || null;
      try {
        await deleteSessionApi(sessionId);

        clearTransportSession(sessionId);

        removeStoredSession(sessionId);
        removePreview(sessionId);
        reconcileSubscriptions();

        if (selectedSessionRef.current?.id === sessionId) {
          const replacement = Object.values(sessionsRef.current)
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
      clearTransportSession,
      reconcileSubscriptions,
      removeStoredSession,
      removePreview,
      sessionsRef,
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
    pendingAssistant: selectedSessionId ? pendingBySessionId[selectedSessionId] || null : null,
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
