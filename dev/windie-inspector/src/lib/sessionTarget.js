export function currentSessionHead(session) {
  return session?.currentHeadMessageId || null;
}
