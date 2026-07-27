import { nextSessionEventCursor } from "./sessionEventCursor";

describe("nextSessionEventCursor", () => {
  test("accepts a new numeric event ID", () => {
    expect(nextSessionEventCursor(4, "5")).toEqual({ accepted: true, cursor: 5 });
  });

  test("rejects duplicate and stale event IDs", () => {
    expect(nextSessionEventCursor(5, 5)).toEqual({ accepted: false, cursor: 5 });
    expect(nextSessionEventCursor(5, 4)).toEqual({ accepted: false, cursor: 5 });
  });

  test("rejects invalid event IDs without changing the cursor", () => {
    expect(nextSessionEventCursor(5, "not-an-id")).toEqual({
      accepted: false,
      cursor: 5,
    });
  });

  test("processes events without an ID without changing the cursor", () => {
    expect(nextSessionEventCursor(5, null)).toEqual({ accepted: true, cursor: 5 });
  });
});
