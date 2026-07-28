import {
  initialSessionState,
  mergeSessionSnapshot,
  sessionStoreReducer,
} from "./sessionStore";

function session(overrides = {}) {
  return {
    id: "session-1",
    latestEventId: 3,
    status: "running",
    ...overrides,
  };
}

test("keeps a newer event cursor when an older snapshot arrives", () => {
  const current = session({ latestEventId: 8 });
  const older = session({ latestEventId: 7, status: "ready" });

  expect(mergeSessionSnapshot(current, older)).toEqual({
    ...older,
    latestEventId: 8,
  });
});

test("reducer stores and removes authoritative session snapshots", () => {
  const running = session({ status: "running" });
  const completed = session({ status: "completed", latestEventId: 4 });
  const withRunning = sessionStoreReducer(initialSessionState, {
    type: "merge",
    session: running,
  });
  const withCompleted = sessionStoreReducer(withRunning, {
    type: "merge",
    session: completed,
  });

  expect(withCompleted.sessionsById["session-1"]).toEqual(completed);
  expect(
    sessionStoreReducer(withCompleted, {
      type: "remove",
      sessionId: "session-1",
    })
  ).toEqual(initialSessionState);
});
