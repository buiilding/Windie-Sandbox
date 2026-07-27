export function currentSessionHead(session) {
  return session?.currentHeadMessageId || null;
}

export function sessionAtHead(sessions, conversationId, headMessageId) {
  if (!conversationId || !headMessageId) return null;
  return (sessions || []).find(
    (session) =>
      session?.conversationId === conversationId &&
      currentSessionHead(session) === headMessageId
  ) || null;
}

export function resolveSessionTarget({
  session,
  conversationId,
  viewHeadId,
  fallbackHead = null,
  action,
}) {
  const sameConversation = session?.conversationId === conversationId;
  const sessionHead = currentSessionHead(session);
  const branchHead = viewHeadId || null;
  const needsNewSession = Boolean(sameConversation && branchHead && branchHead !== sessionHead);

  if (!session || !sameConversation || needsNewSession) {
    return {
      kind: "create",
      headMessageId: branchHead || (sameConversation ? sessionHead : null) || fallbackHead || null,
    };
  }

  return {
    kind: action,
    sessionId: session.id,
    headMessageId: sessionHead,
  };
}
