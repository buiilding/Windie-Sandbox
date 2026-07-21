//! Compact topbar conversation picker with a search field and a new-conversation action.

import { useEffect, useMemo, useRef, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { Plus, ChevronDown, Check, X, MoreHorizontal } from "lucide-react";
import { toast } from "sonner";
import FloatingDeleteMenu, { floatingMenuPosition } from "@/components/windie/FloatingDeleteMenu";

function shortId(id) {
  if (!id) return "";
  return id.slice(0, 8);
}

function conversationLabel(conv) {
  if (!conv) return "no conversation";
  return conv.name || `conversation ${shortId(conv.id)}`;
}

export default function ConversationPicker() {
  const {
    conversations,
    activeConv,
    activeConvId,
    selectConversation,
    createConversation,
    deleteConversation,
  } = useWindie();

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [menuConversation, setMenuConversation] = useState(null);
  const rootRef = useRef(null);
  const inputRef = useRef(null);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setMenuConversation(null);
      return;
    }
    if (inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handleClick = (event) => {
      if (!rootRef.current) return;
      if (!rootRef.current.contains(event.target)) {
        setOpen(false);
      }
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

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return conversations;
    return conversations.filter((conv) => conv.id.toLowerCase().includes(q));
  }, [conversations, query]);

  const sorted = useMemo(() => {
    return [...filtered].sort((a, b) => {
      if (a.id === activeConvId) return -1;
      if (b.id === activeConvId) return 1;
      const aTime = new Date(a.updatedAt || 0).getTime();
      const bTime = new Date(b.updatedAt || 0).getTime();
      return bTime - aTime;
    });
  }, [filtered, activeConvId]);

  const handleCreate = async () => {
    const id = await createConversation();
    if (id) {
      toast.message("new conversation created", { description: shortId(id) });
      setOpen(false);
    }
  };

  const handleSelect = (id) => {
    selectConversation(id);
    setOpen(false);
  };

  const handleDelete = async (event, conversationId) => {
    event.stopPropagation();
    if (!window.confirm("Delete this conversation and its tree?")) return;
    try {
      await deleteConversation(conversationId);
      setMenuConversation(null);
      setOpen(false);
      toast.message("conversation deleted");
    } catch (error) {
      toast.error(error.message);
    }
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        data-testid="topbar-conv-picker"
        onClick={() => setOpen((current) => !current)}
        className={`flex items-center gap-1.5 h-7 px-2 border border-border hover:bg-surface-hover transition-colors min-w-[160px] ${
          open ? "bg-surface-hover" : ""
        }`}
        title={activeConv ? activeConv.id : "no conversation selected"}
      >
        <span className="truncate font-mono text-[11px]">{conversationLabel(activeConv)}</span>
        <ChevronDown className="size-3 ml-auto" strokeWidth={1.75} />
      </button>

      {open && (
        <div
          data-testid="topbar-conv-picker-menu"
          className="absolute right-0 top-full mt-1 z-30 w-72 bg-popover border border-border shadow-md"
        >
          <div className="flex items-center gap-1.5 px-2 h-8 border-b border-border">
            <input
              ref={inputRef}
              data-testid="topbar-conv-picker-search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="filter by id"
              className="flex-1 bg-transparent outline-none font-mono text-[11px] placeholder:text-muted-foreground/60"
            />
            {query && (
              <button
                type="button"
                onClick={() => {
                  setQuery("");
                  if (inputRef.current) inputRef.current.focus();
                }}
                aria-label="clear search"
                className="p-0.5 text-muted-foreground hover:text-foreground"
              >
                <X className="size-3" strokeWidth={1.75} />
              </button>
            )}
          </div>

          <div className="max-h-64 overflow-y-auto windie-scroll" data-testid="topbar-conv-picker-list">
            {sorted.length === 0 ? (
              <div className="px-3 py-3 font-mono text-[11px] text-muted-foreground">
                {query ? "no matches" : "no conversations"}
              </div>
            ) : (
              sorted.map((conv) => {
                const active = conv.id === activeConvId;
                return (
                  <div
                    key={conv.id}
                    className={`relative w-full px-3 py-1.5 font-mono text-[11px] flex items-center gap-1 hover:bg-surface-hover ${
                      active ? "bg-surface" : ""
                    }`}
                  >
                    <button
                      type="button"
                      onClick={() => handleSelect(conv.id)}
                      className="min-w-0 flex-1 text-left flex items-center gap-2"
                    >
                      <span className="truncate">{shortId(conv.id)}</span>
                      <span className="text-muted-foreground truncate flex-1">{conv.model}</span>
                      {active && <Check className="size-3 text-foreground" strokeWidth={2} />}
                    </button>
                    <button
                      type="button"
                      data-testid={`topbar-conv-menu-${shortId(conv.id)}`}
                      aria-label={`conversation actions ${shortId(conv.id)}`}
                      title="conversation actions"
                      onClick={(event) => {
                        event.stopPropagation();
                        const position = floatingMenuPosition(
                          event.currentTarget.getBoundingClientRect()
                        );
                        setMenuConversation((current) =>
                          current?.id === conv.id
                            ? null
                            : {
                                id: conv.id,
                                position,
                              }
                        );
                      }}
                      className="shrink-0 p-1 text-muted-foreground hover:text-foreground hover:bg-surface-hover"
                    >
                      <MoreHorizontal className="size-3.5" strokeWidth={1.75} />
                    </button>
                  </div>
                );
              })
            )}
          </div>

          <div className="border-t border-border">
            <button
              type="button"
              data-testid="topbar-conv-picker-new"
              onClick={handleCreate}
              className="w-full text-left px-3 py-2 font-mono text-[11px] flex items-center gap-2 hover:bg-surface-hover"
            >
              <Plus className="size-3" strokeWidth={1.75} />
              <span className="uppercase tracking-widest">new conversation</span>
            </button>
          </div>
        </div>
      )}
      <FloatingDeleteMenu
        open={Boolean(open && menuConversation)}
        position={menuConversation?.position}
        testId={
          menuConversation
            ? `topbar-conv-delete-${shortId(menuConversation.id)}`
            : "topbar-conv-delete"
        }
        label="delete conversation"
        onDelete={(event) => handleDelete(event, menuConversation?.id)}
      />
    </div>
  );
}
