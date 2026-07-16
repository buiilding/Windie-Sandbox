import { useWindie } from "@/context/WindieContext";
import { Sun, Moon, GitBranch, Columns2 } from "lucide-react";
import ConversationPicker from "@/components/windie/ConversationPicker";

function formatTokenCount(value) {
  if (value == null) return "--";
  if (value >= 1_000_000) return `${Number(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}m`;
  if (value >= 1_000) return `${Number(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return String(value);
}

const TOKEN_METER_TITLE = "Token count over selected model context";

export default function TopBar() {
  const {
    theme,
    setTheme,
    treeOverlayOpen,
    setTreeOverlayOpen,
    inspectorPanelOpen,
    setInspectorPanelOpen,
    tokenMeter,
  } =
    useWindie();

  return (
    <header
      data-testid="windie-topbar"
      className="h-9 shrink-0 border-b border-border flex items-center px-3 gap-4 text-xs font-mono select-none"
    >
      <div className="flex items-center gap-2">
        <div className="size-2 bg-foreground" />
        <span className="font-semibold tracking-tight text-sm font-sans">windie</span>
        <span className="text-muted-foreground">/ local runtime</span>
      </div>

      <div className="h-4 w-px bg-border" />

      <ConversationPicker />

      <div className="flex-1" />

      <div
        className="flex items-center gap-1.5 text-muted-foreground"
        title={TOKEN_METER_TITLE}
      >
        <span className="uppercase tracking-widest">tokens</span>
        <span className="text-foreground">
          {formatTokenCount(tokenMeter?.used)} / {formatTokenCount(tokenMeter?.max)}
        </span>
      </div>

      <button
        data-testid="topbar-toggle-tree"
        onClick={() => setTreeOverlayOpen(!treeOverlayOpen)}
        className={`flex items-center gap-1.5 px-2 py-1 border border-border hover:bg-surface-hover transition-colors ${treeOverlayOpen ? "bg-foreground text-background hover:bg-foreground" : ""}`}
      >
        <GitBranch className="size-3.5" strokeWidth={1.75} />
        <span className="uppercase tracking-widest">tree</span>
      </button>

      <button
        data-testid="topbar-toggle-inspector"
        onClick={() => setInspectorPanelOpen(!inspectorPanelOpen)}
        title={inspectorPanelOpen ? "hide inspector panel" : "show inspector panel"}
        aria-label={inspectorPanelOpen ? "hide inspector panel" : "show inspector panel"}
        className={`flex items-center justify-center size-7 border border-border hover:bg-surface-hover transition-colors ${inspectorPanelOpen ? "bg-foreground text-background hover:bg-foreground" : ""}`}
      >
        <Columns2 className="size-3.5" strokeWidth={1.75} />
      </button>

      <button
        data-testid="topbar-toggle-theme"
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
        aria-label="toggle theme"
        className="flex items-center justify-center size-7 border border-border hover:bg-surface-hover transition-colors"
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
