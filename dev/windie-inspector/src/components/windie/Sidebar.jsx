import { useWindie } from "@/context/WindieContext";
import TreePanel from "@/components/windie/TreePanel";

export default function Sidebar({ treeCollapsed }) {
  const { activeConv } = useWindie();

  return (
    <aside
      data-testid="windie-sidebar"
      className={`shrink-0 border-r border-border flex flex-col bg-background overflow-hidden transition-[width] duration-300 ease-out ${treeCollapsed ? "w-10" : "w-[24.5rem]"}`}
    >
      {!treeCollapsed && (
        <>
          <div className="flex-1 min-h-0" data-testid="windie-sidebar-content">
            {activeConv ? <TreePanel /> : null}
          </div>

          <div className="border-t border-border px-3 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground flex items-center justify-between">
            <span>local</span>
            <span>·</span>
            <span>sqlite tree.db</span>
          </div>
        </>
      )}
    </aside>
  );
}
