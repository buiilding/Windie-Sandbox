import { useWindie } from "@/context/WindieContext";
import { Sun, Moon, GitBranch, Circle, Play, Square } from "lucide-react";

function formatTokenCount(value) {
  if (value == null) return "--";
  if (value >= 1_000_000) return `${Number(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}m`;
  if (value >= 1_000) return `${Number(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return String(value);
}

function tokenMeterTitle(source) {
  if (source === "prequery_input") {
    return "Current input token count over selected model context";
  }
  if (source === "prequery_synthetic_input") {
    return "Approximate current token count using synthetic input";
  }
  return "Token count over selected model context";
}

export default function TopBar() {
  const {
    theme,
    setTheme,
    treeOverlayOpen,
    setTreeOverlayOpen,
    activeConv,
    streaming,
    gatewayRunning,
    tokenMeter,
    startGateway,
    stopGateway,
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

      <div className="flex items-center gap-2 text-muted-foreground min-w-0">
        <span className="uppercase tracking-widest">conv</span>
        <span
          data-testid="topbar-active-conv-name"
          className="truncate text-foreground max-w-[420px]"
          title={activeConv?.name}
        >
          {activeConv?.name}
        </span>
      </div>

      <div className="flex-1" />

      <div
        className="flex items-center gap-1.5 text-muted-foreground"
        title={tokenMeterTitle(tokenMeter?.source)}
      >
        <span className="uppercase tracking-widest">tokens</span>
        <span className="text-foreground">
          {formatTokenCount(tokenMeter?.used)} / {formatTokenCount(tokenMeter?.max)}
        </span>
      </div>

      <div className="h-4 w-px bg-border" />

      <div className="flex items-center gap-1 text-muted-foreground">
        <Circle
          className={`size-2 ${gatewayRunning ? "fill-[hsl(var(--accent))] text-[hsl(var(--accent))]" : "fill-muted-foreground/40 text-muted-foreground/40"}`}
        />
        <span data-testid="topbar-runtime-status" className="uppercase tracking-widest">
          {streaming ? "querying" : gatewayRunning ? "gateway" : "offline"}
        </span>
      </div>

      <button
        data-testid="topbar-gateway-toggle"
        onClick={() => (gatewayRunning ? stopGateway() : startGateway())}
        className="flex items-center justify-center size-7 border border-border hover:bg-surface-hover transition-colors"
        aria-label={gatewayRunning ? "stop gateway" : "start gateway"}
        title={gatewayRunning ? "stop gateway" : "start gateway"}
      >
        {gatewayRunning ? (
          <Square className="size-3" strokeWidth={1.75} />
        ) : (
          <Play className="size-3.5" strokeWidth={1.75} />
        )}
      </button>

      <button
        data-testid="topbar-toggle-tree"
        onClick={() => setTreeOverlayOpen(!treeOverlayOpen)}
        className={`flex items-center gap-1.5 px-2 py-1 border border-border hover:bg-surface-hover transition-colors ${treeOverlayOpen ? "bg-foreground text-background hover:bg-foreground" : ""}`}
      >
        <GitBranch className="size-3.5" strokeWidth={1.75} />
        <span className="uppercase tracking-widest">tree</span>
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
