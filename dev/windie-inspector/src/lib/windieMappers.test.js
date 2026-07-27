import { upsertConversationMessage } from "./windieMappers";

describe("upsertConversationMessage", () => {
  test("adds an authoritative child and advances the selected path", () => {
    const conversation = {
      id: "conversation-1",
      model: "openai/test",
      rootId: "root",
      rootIds: ["root"],
      selectedPath: ["root"],
      nodes: {
        root: {
          id: "root",
          parentId: null,
          childrenIds: [],
          message: { role: "user", parts: [{ type: "text", text: "hello" }] },
        },
      },
    };

    const updated = upsertConversationMessage(
      conversation,
      {
        id: "assistant-1",
        parent_message_id: "root",
        role: "assistant",
        content: "saved answer",
        parts: [{ type: "text", text: "saved answer" }],
        metadata: null,
      },
      "openai/test",
      true
    );

    expect(updated.nodes["assistant-1"].parentId).toBe("root");
    expect(updated.nodes.root.childrenIds).toEqual(["assistant-1"]);
    expect(updated.selectedPath).toEqual(["root", "assistant-1"]);
    expect(updated.nodes["assistant-1"].message.parts[0].text).toBe("saved answer");
  });
});
