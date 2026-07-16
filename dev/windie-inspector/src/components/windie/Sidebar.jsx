import { useWindie } from "@/context/WindieContext";
import TreePanel from "@/components/windie/TreePanel";

export default function Sidebar() {
  const { activeConv } = useWindie();

  return (
    <aside
      data-testid="windie-sidebar"
      className="w-[24.5rem] shrink-0 border-r border-border flex flex-col bg-background"
    >
      <div className="flex-1 min-h-0" data-testid="windie-sidebar-content">
        {activeConv ? <TreePanel /> : null}
      </div>

      <div className="border-t border-border px-3 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground flex items-center justify-between">
        <span>local</span>
        <span>·</span>
        <span>sqlite tree.db</span>
      </div>
    </aside>
  );
}
