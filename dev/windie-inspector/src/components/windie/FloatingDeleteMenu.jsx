import { createPortal } from "react-dom";
import { Trash2 } from "lucide-react";

export function floatingMenuPosition(rect, width = 176) {
  const left = Math.min(
    Math.max(8, rect.right - width),
    Math.max(8, window.innerWidth - width - 8)
  );
  const below = rect.bottom + 4;
  const top = below + 44 <= window.innerHeight ? below : rect.top - 48;
  return { left, top };
}

export default function FloatingDeleteMenu({
  open,
  position,
  testId,
  onDelete,
  label,
}) {
  if (!open || !position) return null;

  return createPortal(
    <div
      data-testid={testId}
      onMouseDown={(event) => event.stopPropagation()}
      className="fixed z-[100] w-44 border border-border bg-popover shadow-md"
      style={{ left: position.left, top: position.top }}
    >
      <button
        type="button"
        onClick={onDelete}
        className="w-full px-3 py-2 flex items-center gap-2 text-left font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))] hover:bg-surface-hover"
      >
        <Trash2 className="size-3" />
        {label}
      </button>
    </div>,
    document.body
  );
}
