import { useWindie } from "@/context/WindieContext";
import ConversationPicker from "@/components/windie/ConversationPicker";
import TreePanel from "@/components/windie/TreePanel";

export default function Sidebar({ treeCollapsed }) {
  const { activeConv } = useWindie();

  return (
    <aside
      data-testid="windie-sidebar"
      className={`shrink-0 flex flex-col bg-background transition-[width] duration-300 ease-out ${treeCollapsed ? "w-0 border-r-0 overflow-hidden" : "w-[24.5rem] border-r border-border overflow-visible"}`}
    >
      {!treeCollapsed && (
        <>
          <div className="relative z-40 shrink-0">
            <ConversationPicker variant="sidebar" />
          </div>
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
