import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { GitBranch, Scissors, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useWindie } from "@/context/WindieContext";

export function treeContextMenuPosition(clientX, clientY) {
  const width = 176;
  const height = 132;
  return {
    left: Math.min(clientX, Math.max(8, window.innerWidth - width - 8)),
    top:
      clientY + height <= window.innerHeight
        ? clientY
        : Math.max(8, clientY - height),
  };
}

export default function TreeNodeContextMenu({ nodeId, position, onClose }) {
  const { activeConv, forkFromMessage, truncateAfter, removeMessage } = useWindie();
  const menuRef = useRef(null);

  useEffect(() => {
    if (!nodeId) return undefined;
    const closeOnOutsideClick = (event) => {
      if (!menuRef.current?.contains(event.target)) onClose();
    };
    const closeOnEscape = (event) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [nodeId, onClose]);

  if (!nodeId || !position || !activeConv) return null;

  const run = (action, message) => {
    onClose();
    try {
      Promise.resolve(action(activeConv.id, nodeId)).catch(() => {});
    } catch (_) {
      // The mutation already reports its own error through the runtime context.
    }
    toast.message(message);
  };

  return createPortal(
    <div
      ref={menuRef}
      data-testid="tree-node-context-menu"
      onMouseDown={(event) => event.stopPropagation()}
      className="fixed z-[100] w-44 border border-border bg-popover shadow-md"
      style={{ left: position.left, top: position.top }}
    >
      <button
        type="button"
        data-testid="tree-node-context-fork"
        onClick={() => run(forkFromMessage, "forked")}
        className="w-full px-3 py-2 flex items-center gap-2 text-left font-mono text-[10px] uppercase tracking-widest hover:bg-surface-hover"
      >
        <GitBranch className="size-3" />
        fork
      </button>
      <button
        type="button"
        data-testid="tree-node-context-truncate"
        onClick={() => run(truncateAfter, "truncated")}
        className="w-full px-3 py-2 flex items-center gap-2 text-left font-mono text-[10px] uppercase tracking-widest hover:bg-surface-hover"
      >
        <Scissors className="size-3" />
        truncate
      </button>
      <button
        type="button"
        data-testid="tree-node-context-remove"
        onClick={() => run(removeMessage, "removed")}
        className="w-full px-3 py-2 flex items-center gap-2 text-left font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))] hover:bg-surface-hover"
      >
        <Trash2 className="size-3" />
        remove
      </button>
    </div>,
    document.body
  );
}
