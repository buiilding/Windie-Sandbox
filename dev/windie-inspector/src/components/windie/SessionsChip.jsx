import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, MoreHorizontal } from "lucide-react";
import { useWindie } from "@/context/WindieContext";
import FloatingDeleteMenu, { floatingMenuPosition } from "@/components/windie/FloatingDeleteMenu";

function shortId(id) {
  return id ? id.slice(0, 8) : "";
}

function statusLabel(status) {
  if (status === "ready") return "ready";
  if (status === "running") return "running";
  if (status === "waiting_for_approval") return "needs approval";
  return status || "unknown";
}

function statusDot(status) {
  if (status === "running") return "bg-green-500";
  if (status === "waiting_for_approval") return "bg-amber-500";
  if (status === "failed") return "bg-red-500";
  return "bg-muted-foreground";
}

function sessionNodeCount(session, nodes) {
  const startId = session.startHeadMessageId;
  let currentId = session.currentHeadMessageId || session.startHeadMessageId;
  let count = 0;
  const visited = new Set();

  while (currentId && !visited.has(currentId)) {
    visited.add(currentId);
    if (currentId === startId) break;
    count += 1;
    currentId = nodes[currentId]?.parentId || null;
  }

  return count;
}

export default function SessionsChip({ dropUp = false }) {
  const {
    activeConv,
    sessionsById,
    selectedSessionId,
    viewHeadId,
    selectSession,
    deleteSession,
  } = useWindie();
  const [open, setOpen] = useState(false);
  const [menuSession, setMenuSession] = useState(null);
  const rootRef = useRef(null);

  const sessions = useMemo(() => {
    if (!activeConv) return [];
    return Object.values(sessionsById)
      .filter((session) => session.conversationId === activeConv.id)
      .sort((a, b) => (b.updatedAt || b.createdAt || 0) - (a.updatedAt || a.createdAt || 0));
  }, [activeConv, sessionsById]);

  useEffect(() => {
    if (!open) {
      setMenuSession(null);
      return;
    }
    const handleClick = (event) => {
      if (rootRef.current && !rootRef.current.contains(event.target)) setOpen(false);
    };
    const handleKey = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKey);
    };
  }, [open]);

  if (!activeConv || sessions.length === 0) return null;

  const selected = viewHeadId
    ? null
    : sessions.find((session) => session.id === selectedSessionId) || null;

  const handleDelete = async (event, session) => {
    event.stopPropagation();
    if (!session) return;
    const deleted = await deleteSession(session.id);
    if (deleted) setMenuSession(null);
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        data-testid="topbar-sessions-chip"
        onClick={() => setOpen((current) => !current)}
        className={`flex items-center gap-1.5 h-7 px-2 border border-border bg-background hover:bg-surface-hover transition-colors min-w-[160px] ${open ? "bg-surface-hover" : ""}`}
        title={selected ? `session ${selected.id}` : viewHeadId ? "new session at selected path" : "choose a session"}
      >
        {selected && <span className={`size-1.5 rounded-full ${statusDot(selected.status)}`} />}
        <span className="truncate font-mono text-[11px]">
          {selected ? `session ${shortId(selected.id)}` : viewHeadId ? "new session" : "choose session"}
        </span>
        <ChevronDown className="size-3 ml-auto" strokeWidth={1.75} />
      </button>

      {open && (
        <div
          data-testid="topbar-sessions-menu"
          className={`absolute z-30 w-72 bg-popover border border-border shadow-md ${dropUp ? "left-0 bottom-full mb-1" : "left-0 top-full mt-1"}`}
        >
          <div className="px-2.5 py-1.5 border-b border-border font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            sessions · {sessions.length}
          </div>
          <div className="max-h-64 overflow-y-auto windie-scroll">
            {sessions.length === 0 ? (
              <div className="px-3 py-3 font-mono text-[11px] text-muted-foreground">
                no sessions yet
              </div>
            ) : (
              sessions.map((session) => (
                <div
                  key={session.id}
                  className="relative w-full px-3 py-2 font-mono text-[11px] flex items-center gap-1 hover:bg-surface-hover"
                >
                  <button
                    type="button"
                    data-testid={`topbar-session-${shortId(session.id)}`}
                    onClick={() => {
                      selectSession(session.id);
                      setOpen(false);
                    }}
                    className="min-w-0 flex-1 text-left flex items-center gap-2"
                  >
                    <span className={`size-1.5 rounded-full shrink-0 ${statusDot(session.status)}`} />
                    <span className="shrink-0">{shortId(session.id)}</span>
                    {session.status !== "completed" && (
                      <span className="text-muted-foreground uppercase text-[10px] shrink-0">
                        {statusLabel(session.status)}
                      </span>
                    )}
                    <span className="truncate flex-1 text-muted-foreground text-right text-[10px]">
                      {sessionNodeCount(session, activeConv.nodes)} nodes
                    </span>
                    {!viewHeadId && session.id === selectedSessionId && <Check className="size-3 shrink-0" />}
                  </button>
                  <button
                    type="button"
                    data-testid={`topbar-session-menu-${shortId(session.id)}`}
                    aria-label={`session actions ${shortId(session.id)}`}
                    title="session actions"
                    onClick={(event) => {
                      event.stopPropagation();
                      const position = floatingMenuPosition(
                        event.currentTarget.getBoundingClientRect()
                      );
                      setMenuSession((current) =>
                        current?.id === session.id
                          ? null
                          : {
                              id: session.id,
                              position,
                            }
                      );
                    }}
                    className="shrink-0 p-1 text-muted-foreground hover:text-foreground hover:bg-surface-hover"
                  >
                    <MoreHorizontal className="size-3.5" strokeWidth={1.75} />
                  </button>
                </div>
              ))
            )}
          </div>
        </div>
      )}
      <FloatingDeleteMenu
        open={Boolean(open && menuSession)}
        position={menuSession?.position}
        testId={
          menuSession
            ? `topbar-session-delete-${shortId(menuSession.id)}`
            : "topbar-session-delete"
        }
        label="delete session"
        onDelete={(event) =>
          handleDelete(
            event,
            sessions.find((session) => session.id === menuSession?.id)
          )
        }
      />
    </div>
  );
}
