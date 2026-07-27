import { useCallback, useEffect, useRef } from "react";
import { streamSessionEvents } from "@/lib/sessionStream";
import { nextSessionEventCursor } from "@/lib/sessionEventCursor";
import { isLiveSession } from "@/lib/sessionState";

function isAbortError(error) {
  return error?.name === "AbortError";
}

/**
 * Owns the browser transport for durable session events.
 *
 * This hook deliberately knows nothing about message rendering, approvals, or
 * conversation selection. It tracks SSE controllers and replay cursors, then
 * forwards each accepted event to the workflow hook.
 */
export function useSessionTransport({ sessionsRef, rememberSession, onEvent, onError }) {
  const subscriptionsRef = useRef(new Map());
  const lastEventIdRef = useRef({});
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

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
    if (!current || next.cursor == null) return true;
    rememberSession({ ...current, latestEventId: next.cursor });
    return true;
  }, [rememberSession, sessionsRef]);

  const unsubscribe = useCallback((sessionId) => {
    const controller = subscriptionsRef.current.get(sessionId);
    if (controller) controller.abort();
    subscriptionsRef.current.delete(sessionId);
  }, []);

  const subscribe = useCallback((session) => {
    if (!session || subscriptionsRef.current.has(session.id)) return session || null;

    const controller = new AbortController();
    subscriptionsRef.current.set(session.id, controller);
    streamSessionEvents(
      session.id,
      lastEventIdRef.current[session.id] ?? null,
      async ({ id, data }) => {
        const eventId = id ?? data?.event_id ?? null;
        const accepted = advanceSessionEventCursor(session.id, eventId);
        if (eventId != null && !accepted) return;
        const current = sessionsRef.current[session.id] || session;
        await onEventRef.current(current, data);
      },
      { signal: controller.signal }
    )
      .catch((error) => {
        if (!isAbortError(error)) onErrorRef.current(error);
      })
      .finally(() => {
        if (subscriptionsRef.current.get(session.id) === controller) {
          subscriptionsRef.current.delete(session.id);
        }
      });

    return session;
  }, [advanceSessionEventCursor, sessionsRef]);

  const reconcileSubscriptions = useCallback(() => {
    const liveSessions = Object.values(sessionsRef.current).filter(isLiveSession);
    const liveIds = new Set(liveSessions.map((session) => session.id));

    for (const session of liveSessions) subscribe(session);
    for (const sessionId of subscriptionsRef.current.keys()) {
      if (!liveIds.has(sessionId)) unsubscribe(sessionId);
    }
  }, [sessionsRef, subscribe, unsubscribe]);

  useEffect(() => {
    reconcileSubscriptions();
  }, [reconcileSubscriptions]);

  useEffect(
    () => () => {
      for (const controller of subscriptionsRef.current.values()) controller.abort();
      subscriptionsRef.current.clear();
    },
    []
  );

  const clearSession = useCallback((sessionId) => {
    unsubscribe(sessionId);
    delete lastEventIdRef.current[sessionId];
  }, [unsubscribe]);

  return {
    hydrateSessionEventCursor,
    subscribe,
    unsubscribe,
    reconcileSubscriptions,
    clearSession,
  };
}
