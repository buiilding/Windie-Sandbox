import { currentSessionHead } from "../lib/sessionTarget";

describe("currentSessionHead", () => {
  test("returns the durable current head", () => {
    expect(currentSessionHead({ currentHeadMessageId: "head-2" })).toBe("head-2");
  });

  test("returns null when a session has no current head", () => {
    expect(currentSessionHead({ currentHeadMessageId: null })).toBeNull();
  });
});
