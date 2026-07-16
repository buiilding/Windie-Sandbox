import { useWindie } from "@/context/WindieContext";

export default function Sidebar() {
  const { activeConv } = useWindie();

  return (
    <aside
      data-testid="windie-sidebar"
      className="w-64 shrink-0 border-r border-border flex flex-col bg-background"
    >
      <div className="flex-1 min-h-0" data-testid="windie-sidebar-content">
        {activeConv ? (
          <div className="p-4 font-mono text-xs text-muted-foreground">
            tree moves here next
          </div>
        ) : (
          <div className="p-4 font-mono text-xs text-muted-foreground">
            no conversation
          </div>
        )}
      </div>

      <div className="border-t border-border px-3 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground flex items-center justify-between">
        <span>local</span>
        <span>·</span>
        <span>sqlite tree.db</span>
      </div>
    </aside>
  );
}
