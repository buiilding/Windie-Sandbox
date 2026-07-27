/**
 * Small browser-side session state helpers.
 *
 * These helpers describe API state for presentation and persistence of the
 * selected session. They do not decide runtime policy.
 */

export function isLiveSession(session) {
  return session?.status === "running" || session?.status === "waiting_for_approval";
}

const SELECTED_SESSION_STORAGE_PREFIX = "windie.selected-session:";

function selectedSessionStorageKey(conversationId) {
  return `${SELECTED_SESSION_STORAGE_PREFIX}${conversationId || ""}`;
}

export function readSelectedSessionId(conversationId) {
  if (!conversationId) return null;
  try {
    return window.localStorage.getItem(selectedSessionStorageKey(conversationId));
  } catch (_) {
    return null;
  }
}

export function writeSelectedSessionId(conversationId, sessionId) {
  if (!conversationId || !sessionId) return;
  try {
    window.localStorage.setItem(selectedSessionStorageKey(conversationId), sessionId);
  } catch (_) {
    // Browser storage is optional; in-memory selection still works.
  }
}
