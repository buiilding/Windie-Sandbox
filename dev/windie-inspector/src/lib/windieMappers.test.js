import { upsertConversationMessage } from "./windieMappers";
import { providerInstallationsFromApi } from "./windieMappers";

describe("providerInstallationsFromApi", () => {
  test("preserves actionable provider readiness", () => {
    const [provider] = providerInstallationsFromApi([
      {
        manifest: { provider_id: "brightdata", display_name: "Bright Data", runtime: "node" },
        installation: {
          state: "broken",
          readiness: "missing_secret",
          next_action: "configure the provider secret and repair",
          error: "missing required provider secret BRIGHTDATA_API_TOKEN",
        },
      },
    ]);

    expect(provider.installation.readiness).toBe("missing_secret");
    expect(provider.installation.nextAction).toContain("configure");
    expect(provider.installation.error).toContain("BRIGHTDATA");
    expect(provider.runtime).toBe("node");
  });
});

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
