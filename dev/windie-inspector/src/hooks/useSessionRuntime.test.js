import { resolveSessionTarget } from "../lib/sessionTarget";

describe("resolveSessionTarget", () => {
  const session = {
    id: "session-1",
    conversationId: "conversation-1",
    currentHeadMessageId: "head-2",
  };

  test("reuses the selected session at its current head", () => {
    expect(
      resolveSessionTarget({
        session,
        conversationId: "conversation-1",
        action: "query",
      })
    ).toEqual({
      kind: "query",
      sessionId: "session-1",
      headMessageId: "head-2",
    });
  });

  test("creates a session when sending from a historical viewed head", () => {
    expect(
      resolveSessionTarget({
        session,
        conversationId: "conversation-1",
        viewHeadId: "head-1",
        fallbackHead: "head-2",
        action: "query",
      })
    ).toEqual({
      kind: "create",
      headMessageId: "head-1",
    });
  });

  test("does not carry a stale session head across conversations", () => {
    expect(
      resolveSessionTarget({
        session,
        conversationId: "conversation-2",
        fallbackHead: "head-9",
        action: "continue",
      })
    ).toEqual({
      kind: "create",
      headMessageId: "head-9",
    });
  });
});
