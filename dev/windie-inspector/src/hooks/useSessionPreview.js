import { useCallback, useState } from "react";

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
 * Owns only transient rendering state for one or more live sessions.
 * Persisted messages and session status remain backend snapshots.
 */
export function useSessionPreview() {
  const [pendingBySessionId, setPendingBySessionId] = useState({});

  const startTurn = useCallback((session) => {
    setPendingBySessionId((current) => ({
      ...current,
      [session.id]: emptyPending(session),
    }));
  }, []);

  const applyDelta = useCallback((session, event) => {
    setPendingBySessionId((current) => reducePending(current, session, event));
  }, []);

  const applySavedMessage = useCallback((session, eventType) => {
    setPendingBySessionId((current) => {
      const pending = current[session.id];
      if (!pending) return current;

      if (eventType === "tool_result_saved" || Object.keys(pending.toolCalls || {}).length > 0) {
        return { ...current, [session.id]: resetPendingTurn(pending) };
      }

      return { ...current, [session.id]: null };
    });
  }, []);

  const clearSession = useCallback((sessionId) => {
    setPendingBySessionId((current) => ({ ...current, [sessionId]: null }));
  }, []);

  const removeSession = useCallback((sessionId) => {
    setPendingBySessionId((current) => {
      const next = { ...current };
      delete next[sessionId];
      return next;
    });
  }, []);

  return {
    pendingBySessionId,
    startTurn,
    applyDelta,
    applySavedMessage,
    clearSession,
    removeSession,
  };
}
