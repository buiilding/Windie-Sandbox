import { projectSessionEvent } from "./sessionEvent";

const session = {
  id: "session-1",
  conversation_id: "conversation-1",
  status: "running",
  model: "test-model",
};

test("projects authoritative session and saved message snapshots", () => {
  const projection = projectSessionEvent(session, {
    type: "assistant_message_saved",
    session: { ...session, status: "completed" },
    message: { id: "message-1", role: "assistant", content: "done" },
  });

  expect(projection.session.status).toBe("completed");
  expect(projection.message.id).toBe("message-1");
  expect(projection.isSavedMessage).toBe(true);
  expect(projection.isDelta).toBe(false);
});

test("keeps delta events transient and uses the current session", () => {
  const current = { id: "session-1", status: "running" };
  const projection = projectSessionEvent(current, {
    type: "assistant_delta",
    text: "hello",
  });

  expect(projection.session).toBe(current);
  expect(projection.isDelta).toBe(true);
  expect(projection.isSavedMessage).toBe(false);
  expect(projection.isTerminal).toBe(false);
});
