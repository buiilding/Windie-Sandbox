import { useWindie } from "@/context/WindieContext";
import { Sun, Moon, GitBranch } from "lucide-react";
import ConversationPicker from "@/components/windie/ConversationPicker";

function formatTokenCount(value) {
  if (value == null) return "--";
  if (value >= 1_000_000) return `${Number(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}m`;
  if (value >= 1_000) return `${Number(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return String(value);
}

const TOKEN_METER_TITLE = "Token count over selected model context";

export default function TopBar({ treeCollapsed, onTreeToggle, overlay, onOverlayChange }) {
  const {
    theme,
    setTheme,
    tokenMeter,
    approvals,
  } =
    useWindie();

  return (
    <header
      data-testid="windie-topbar"
      className="relative z-50 h-9 shrink-0 pointer-events-none flex items-center px-3 gap-4 bg-background border-b border-border text-xs font-mono select-none"
    >
      <div className="flex items-center gap-2">
        <div className="size-2 bg-foreground" />
        <span className="font-semibold tracking-tight text-sm font-sans">windie</span>
      </div>

      <div className="h-4 w-px bg-border" />

      <button
        type="button"
        data-testid="topbar-toggle-tree"
        onClick={onTreeToggle}
        title={treeCollapsed ? "show conversation tree" : "hide conversation tree"}
        aria-label={treeCollapsed ? "show conversation tree" : "hide conversation tree"}
        className="pointer-events-auto flex items-center justify-center size-7 border border-border bg-background hover:bg-surface-hover transition-colors"
      >
        <GitBranch className="size-3.5" strokeWidth={1.75} />
      </button>

      <div className="pointer-events-auto">
        <ConversationPicker />
      </div>

      <div className="flex-1" />

      <div
        className="flex items-center gap-1.5"
        title={TOKEN_METER_TITLE}
      >
        <button
          type="button"
          data-testid="topbar-open-system"
          onClick={() => onOverlayChange(overlay === "system" ? null : "system")}
          className={`pointer-events-auto h-6 px-1.5 border border-border bg-background font-mono text-[10px] uppercase tracking-widest text-foreground hover:bg-surface-hover transition-colors ${overlay === "system" ? "bg-surface-hover" : ""}`}
        >
          system
        </button>

        <button
          type="button"
          data-testid="topbar-open-tools"
          onClick={() => onOverlayChange(overlay === "tools" ? null : "tools")}
          className={`pointer-events-auto h-6 px-1.5 border border-border bg-background font-mono text-[10px] uppercase tracking-widest text-foreground hover:bg-surface-hover transition-colors ${overlay === "tools" ? "bg-surface-hover" : ""}`}
        >
          tools{approvals.length > 0 ? ` · ${approvals.length}` : ""}
        </button>

        <div className="h-4 w-px bg-border mx-1" />

        <span className="uppercase tracking-widest">tokens</span>
        <span className="text-foreground">
          {formatTokenCount(tokenMeter?.used)} / {formatTokenCount(tokenMeter?.max)}
        </span>
      </div>

      <button
        data-testid="topbar-toggle-theme"
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
        aria-label="toggle theme"
        className="pointer-events-auto flex items-center justify-center size-7 border border-border hover:bg-surface-hover transition-colors"
      >
        {theme === "dark" ? (
          <Sun className="size-3.5" strokeWidth={1.75} />
        ) : (
          <Moon className="size-3.5" strokeWidth={1.75} />
        )}
      </button>
    </header>
  );
}
