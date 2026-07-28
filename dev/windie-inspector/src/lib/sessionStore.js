/**
 * Pure state transitions for the frontend's durable session projection.
 *
 * The backend owns session truth. This module only keeps the latest backend
 * snapshots available to React and to asynchronous transport callbacks.
 */

export const initialSessionState = {
  sessionsById: {},
};

export function mergeSessionSnapshot(existing, session) {
  if (!session) return existing || null;

  if (
    existing?.latestEventId != null &&
    (session.latestEventId == null || session.latestEventId < existing.latestEventId)
  ) {
    return { ...session, latestEventId: existing.latestEventId };
  }

  return session;
}

export function sessionStoreReducer(state, action) {
  switch (action.type) {
    case "replace":
      return {
        sessionsById: Object.fromEntries(
          action.sessions.filter(Boolean).map((session) => [session.id, session])
        ),
      };
    case "merge": {
      const existing = state.sessionsById[action.session.id];
      const session = mergeSessionSnapshot(existing, action.session);
      return {
        sessionsById: {
          ...state.sessionsById,
          [session.id]: session,
        },
      };
    }
    case "remove": {
      if (!state.sessionsById[action.sessionId]) return state;
      const sessionsById = { ...state.sessionsById };
      delete sessionsById[action.sessionId];
      return { sessionsById };
    }
    default:
      return state;
  }
}
