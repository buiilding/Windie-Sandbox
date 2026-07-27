export function nextSessionEventCursor(currentCursor, eventId) {
  if (eventId == null) {
    return { accepted: true, cursor: currentCursor };
  }

  const numericEventId = Number(eventId);
  if (!Number.isFinite(numericEventId)) {
    return { accepted: false, cursor: currentCursor };
  }

  if (currentCursor != null && numericEventId <= currentCursor) {
    return { accepted: false, cursor: currentCursor };
  }

  return { accepted: true, cursor: numericEventId };
}
