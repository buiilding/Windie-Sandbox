import { useCallback, useReducer, useRef } from "react";
import {
  initialSessionState,
  mergeSessionSnapshot,
  sessionStoreReducer,
} from "@/lib/sessionStore";

/**
 * Keeps backend-owned session snapshots in React state and in a synchronous
 * ref for event callbacks that can run between React renders.
 */
export function useSessionStore() {
  const [state, dispatch] = useReducer(sessionStoreReducer, initialSessionState);
  const sessionsRef = useRef({});

  const rememberSession = useCallback((session) => {
    if (!session) return null;
    const merged = mergeSessionSnapshot(sessionsRef.current[session.id], session);
    sessionsRef.current = { ...sessionsRef.current, [merged.id]: merged };
    dispatch({ type: "merge", session: merged });
    return merged;
  }, []);

  const rememberSessions = useCallback((sessions) => {
    const next = { ...sessionsRef.current };
    for (const session of sessions || []) {
      if (!session) continue;
      next[session.id] = mergeSessionSnapshot(next[session.id], session);
    }
    sessionsRef.current = next;
    dispatch({ type: "replace", sessions: Object.values(next) });
    return next;
  }, []);

  const replaceSessions = useCallback((sessions) => {
    const next = Object.fromEntries(
      (sessions || []).filter(Boolean).map((session) => [session.id, session])
    );
    sessionsRef.current = next;
    dispatch({ type: "replace", sessions: Object.values(next) });
    return Object.values(next);
  }, []);

  const removeSession = useCallback((sessionId) => {
    if (!sessionId || !sessionsRef.current[sessionId]) return null;
    const removed = sessionsRef.current[sessionId];
    const next = { ...sessionsRef.current };
    delete next[sessionId];
    sessionsRef.current = next;
    dispatch({ type: "remove", sessionId });
    return removed;
  }, []);

  return {
    sessionsById: state.sessionsById,
    sessionsRef,
    rememberSession,
    rememberSessions,
    replaceSessions,
    removeSession,
  };
}
