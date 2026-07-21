import { useEffect, useRef, useState } from "react";
import { MoreHorizontal, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useWindie } from "@/context/WindieContext";

export default function ConversationTreeMenu() {
  const { activeConv, deleteConversation } = useWindie();
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);

  useEffect(() => {
    if (!open) return undefined;
    const closeOnOutsideClick = (event) => {
      if (!rootRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  if (!activeConv) return null;

  const deleteCurrentConversation = async () => {
    try {
      await deleteConversation(activeConv.id);
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
        data-testid="conversation-tree-menu-toggle"
        aria-label="conversation tree actions"
        title="conversation tree actions"
        onClick={() => setOpen((current) => !current)}
        className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
      >
        <MoreHorizontal className="size-3.5" strokeWidth={1.75} />
      </button>
      {open && (
        <div
          data-testid="conversation-tree-menu"
          className="absolute right-0 top-full mt-1 z-40 w-48 border border-border bg-popover shadow-md"
        >
          <button
            type="button"
            data-testid="conversation-tree-delete"
            onClick={deleteCurrentConversation}
            className="w-full px-3 py-2 flex items-center gap-2 text-left font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))] hover:bg-surface-hover"
          >
            <Trash2 className="size-3" />
            delete conversation
          </button>
        </div>
      )}
    </div>
  );
}
