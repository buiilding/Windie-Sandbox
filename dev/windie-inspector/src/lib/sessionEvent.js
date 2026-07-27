import { sessionFromApi } from "./windieMappers";

const DELTA_EVENT_TYPES = new Set([
  "assistant_delta",
  "reasoning_delta",
  "tool_call_delta",
]);

const SAVED_MESSAGE_EVENT_TYPES = new Set([
  "assistant_message_saved",
  "tool_result_saved",
]);

const TERMINAL_EVENT_TYPES = new Set([
  "completed",
  "failed",
  "cancelled",
  "waiting_for_approval",
]);

/**
 * Converts one backend event into frontend projection instructions.
 *
 * Session snapshots and saved message snapshots are authoritative. Delta
 * events are intentionally left as transient preview data for the renderer.
 */
export function projectSessionEvent(session, data) {
  if (!data?.type) return null;

  return {
    type: data.type,
    session: data.session ? sessionFromApi(data.session) : session,
    message: data.message || null,
    isDelta: DELTA_EVENT_TYPES.has(data.type),
    isSavedMessage: SAVED_MESSAGE_EVENT_TYPES.has(data.type),
    isTerminal: TERMINAL_EVENT_TYPES.has(data.type),
  };
}
