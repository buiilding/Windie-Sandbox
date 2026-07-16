//! Compact topbar conversation picker with a search field and a new-conversation action.

import { useEffect, useMemo, useRef, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { Plus, ChevronDown, Check, X } from "lucide-react";
import { toast } from "sonner";

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
  } = useWindie();

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const rootRef = useRef(null);
  const inputRef = useRef(null);

  useEffect(() => {
    if (!open) {
      setQuery("");
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
                  <button
                    type="button"
                    key={conv.id}
                    onClick={() => handleSelect(conv.id)}
                    className={`w-full text-left px-3 py-1.5 font-mono text-[11px] flex items-center gap-2 hover:bg-surface-hover ${
                      active ? "bg-surface" : ""
                    }`}
                  >
                    <span className="truncate">{shortId(conv.id)}</span>
                    <span className="text-muted-foreground truncate flex-1">{conv.model}</span>
                    {active && <Check className="size-3 text-foreground" strokeWidth={2} />}
                  </button>
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
    </div>
  );
}
